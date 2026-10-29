import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as pause } from "node:timers/promises";
import { test } from "node:test";

async function until(read, description) {
  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) {
    const value = await read();
    if (value) return value;
    await pause(50);
  }
  throw new Error(`等待超时：${description}`);
}

// 使用 GitHub Ubuntu runner 自带 Chrome 和 CDP，无需额外浏览器测试依赖。
test("配置库优先 GUI：首次使用、逐对分发、编辑、导入与持久化", { timeout: 240000 }, async (t) => {
  const preview = spawn(process.execPath, [
    "node_modules/vite/bin/vite.js", "preview", "--host", "127.0.0.1", "--port", "4173", "--strictPort",
  ], { stdio: "ignore" });
  t.after(() => preview.kill());
  await until(async () => {
    try { return (await fetch("http://127.0.0.1:4173")).ok; } catch { return false; }
  }, "前端预览服务");

  const profile = await mkdtemp(join(tmpdir(), "mcphub-gui-test-"));
  const chrome = spawn(process.env.CHROME_PATH || "/usr/bin/google-chrome", [
    "--headless", "--no-sandbox", "--disable-dev-shm-usage", "--remote-debugging-pipe", `--user-data-dir=${profile}`,
  ], { stdio: ["ignore", "ignore", "pipe", "pipe", "pipe"] });
  t.after(() => chrome.kill());
  await once(chrome, "spawn");
  let sequence = 0;
  let buffer = "";
  let diagnostics = "";
  const pending = new Map();
  chrome.stderr.setEncoding("utf8");
  chrome.stderr.on("data", (chunk) => { diagnostics = (diagnostics + chunk).slice(-3000); });
  const failPending = (error) => {
    for (const request of pending.values()) { clearTimeout(request.timer); request.reject(error); }
    pending.clear();
  };
  chrome.once("exit", (code, signal) => failPending(new Error(`Chrome 已退出 (${code ?? signal})：${diagnostics}`)));
  chrome.stdio[3].on("error", failPending);
  chrome.stdio[4].setEncoding("utf8");
  chrome.stdio[4].on("data", (chunk) => {
    buffer += chunk;
    let end;
    while ((end = buffer.indexOf("\0")) !== -1) {
      const message = JSON.parse(buffer.slice(0, end));
      buffer = buffer.slice(end + 1);
      const request = pending.get(message.id);
      if (request) {
        pending.delete(message.id);
        clearTimeout(request.timer);
        if (message.error) request.reject(new Error(message.error.message));
        else request.resolve(message.result);
      }
    }
  });
  function send(method, params = {}, sessionId) {
    return new Promise((resolve, reject) => {
      const id = ++sequence;
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`CDP 超时：${method}；${diagnostics}`));
      }, method === "Browser.getVersion" ? 30000 : 10000);
      pending.set(id, { resolve, reject, timer });
      chrome.stdio[3].write(JSON.stringify({ id, method, params, sessionId }) + "\0");
    });
  }
  // 先等待浏览器启动就绪，冷启动使用独立时限，功能断言仍保持原有等待边界。
  await send("Browser.getVersion");
  const { targetId } = await send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
  const evaluate = async (expression) => {
    const result = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, sessionId);
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  function installFixture() {
    const ids = ["claude-code", "codex"];
    const clients = ids.map((id, i) => ({ id, name: ["Claude Code", "Codex CLI"][i], kind: "CLI" }));
    window.testCalls = [];
    window.manyClients = true;
    window.confirmAnswer = true;
    window.confirm = (message) => { window.lastConfirmation = message; return window.confirmAnswer; };
    window.catalogEntries = [
      { name: "alpha", description: "文件与项目工具", transport: "stdio", command: "python", args: ["one,two", "  keep spaces  "], scope: "inherit", tags: ["tools"], enabled: false, timeout_seconds: 45, overrides: { windows: { command: "python.exe" } } },
      { name: "beta", description: "文档检索服务", transport: "stdio", command: "npx", args: [], scope: "inherit" },
      { name: "project-only", description: "仅用于当前项目", transport: "stdio", command: "python", scope: "project" },
    ];
    window.__TAURI_INTERNALS__ = {
      async invoke(command, args) {
        const activeClients = window.manyClients ? clients.concat(Array.from({ length: 12 }, (_, i) => ({ id: `test-agent-${i}`, name: `Example Agent ${i + 1}`, kind: "IDE" }))) : clients;
        window.testCalls.push({ command, args: structuredClone(args) });
        switch (command) {
          case "load_settings": return JSON.parse(sessionStorage.getItem("fixtureSettings") || "null") || { version: 1, catalogPath: "C:/test/catalog.yaml", backupDir: "C:/test/backups", defaultScope: "user", defaultMode: "merge", lastProjectDir: null };
          case "default_settings": return { version: 1, catalogPath: "C:/test/catalog.yaml", backupDir: "C:/test/backups", defaultScope: "user", defaultMode: "merge", lastProjectDir: null };
          case "save_settings":
            if (window.rejectSettings) throw new Error("测试设置保存失败");
            sessionStorage.setItem("fixtureSettings", JSON.stringify(args.settings)); return [];
          case "choose_directory": return window.cancelDirectory ? null : "C:/test/中文项目";
          case "create_catalog": window.catalogMissing = false; return [];
          case "save_catalog": window.catalogEntries = structuredClone(args.catalog.entries); window.hasBackup = true; return [];
          case "restore": window.restored = true; return 1;
          case "app_info": return { version: "test", platform: "windows", dataDir: "test-only" };
          case "detect": return activeClients.map((c) => ({ ...c, state: c.id.startsWith("test-") ? "missing" : "ready", configured: c.id.startsWith("test-") ? 0 : 1, path: "C:/test/agent/config.json",
            scopeBlock: args.scope === "local" ? "不支持当前作用域" : null }));
          case "read_client_config": {
            if (window.failRead) throw new Error("配置解析失败；未显示原文");
            if (window.delayRead) await new Promise((resolve) => { window.releaseRead = resolve; });
            return {
              clientId: args.clientId, clientName: args.clientId, scope: args.scope,
              path: `${args.clientId}/config.test`, format: "JSON", existed: !window.missingConfig,
              entries: window.missingConfig ? [] : [{ name: `${args.clientId}-已有服务`, config: {
                command: "python", args: ["[已隐藏]"], env: { API_KEY: "[已隐藏]" },
              } }],
            };
          }
          case "matrix": {
            if (window.failMatrix) throw new Error("清单不存在（测试）");
            return {
            platform: "windows", scope: args.scope, clients: activeClients, issues: [],
            rows: window.catalogEntries.filter((e) => !e.scope || e.scope === "inherit" || e.scope === args.scope).map(({name}) => ({
              name, transport: "stdio", scope: args.scope,
              cells: activeClients.map(({ id: clientId }) => ({ clientId, state: "absent", detail: null })),
            })),
            };
          }
          case "load_catalog": {
            if (window.catalogCorrupt) throw new Error("配置库格式无效；原文件未修改");
            const entries = window.catalogMissing ? [] : window.catalogEntries;
            return { exists: !window.catalogMissing, catalog: { version: 1, entries: structuredClone(entries) }, issues: [], entryCount: entries.length };
          }
          case "scan_import": return { scanId: `scan-${window.testCalls.length}`, sourcePath: "C:/test/source/config.toml", catalogPath: args.catalogPath, catalogExists: true,
            items: [
              { name: "alpha", conflict: true, blocked: null, transport: "http", fields: [{ id: "/auth/token_env", label: "Bearer 凭据" }] },
              { name: "new-mcp", conflict: false, blocked: null, transport: "stdio", fields: [{ id: "/args/0", label: "参数 1" }] },
              { name: "unsupported", conflict: false, blocked: "包含无法无损转换的字段", transport: null, fields: [] },
            ] };
          case "preview_import": {
            if (window.failImportPreview) throw new Error("目标配置库已变化，请重新扫描");
            window.importPreview = { previewId: `preview-${window.testCalls.length}`, catalogPath: "C:/test/catalog.yaml",
              changes: args.choices.filter((choice) => choice.action !== "skip").map((choice) => ({ sourceName: choice.sourceName, targetName: choice.targetName, action: choice.action,
                before: choice.action === "replace" ? { command: "[已隐藏]", args: ["[已隐藏]"] } : null,
                after: { name: choice.targetName, transport: "stdio", command: "python", args: Object.values(choice.bindings).map((name) => `\${ENV:${name}}`) },
              })) };
            return window.importPreview;
          }
          case "commit_import":
            if (window.staleImport) throw new Error("源配置已变化，请重新扫描并预览");
            window.importedEntries = window.importPreview.changes.map((change) => change.after); window.catalogEntries.push(...window.importedEntries); window.catalogMissing = false; window.hasBackup = true;
            return { imported: window.importedEntries.length, warnings: ["配置库已保存，审计写入失败（测试）"] };
          case "cancel_import": return null;
          case "backups": return window.hasBackup ? [{ id: "test-backup", createdAt: "2026-10-29T12:00:00Z", files: 1, paths: ["C:/test/catalog.yaml"] }] : [];
          case "plan": {
            if (window.delayPlan) await new Promise((resolve) => { window.releasePlan = resolve; });
            const files = Object.entries(args.selection).map(([clientId, names]) => ({
              clientId, clientName: clientId, path: `C:/test/${args.scope}/${clientId}/mcp.json`, format: "JSON", scope: args.scope,
              existed: false, blocked: null, newCount: names.length, updateCount: 0, unchangedCount: 0,
              diff: `+${names.join(",")}`, warnings: [], willWrite: true,
            }));
            return { entryCount: 2, files, writable: files.length, issues: [], missingVars: [], skipped: [] };
          }
          case "apply": window.hasBackup = true; return window.applyFailure ? {
            written: 1, blocked: 0, failed: 2, backupId: "test-backup",
            outcomes: [
              { clientId: "claude-code", status: "rollbackFailed", message: "自动回滚失败，请检查备份" },
              { clientId: "-", status: "auditFailed", message: "审计写入失败，文件未回滚" },
            ],
          } : { written: 2, blocked: 0, failed: 0, backupId: null, outcomes: [] };
          default: throw new Error(`未预期的 IPC：${command}`);
        }
      },
    };
  }
  await send("Page.enable", {}, sessionId);
  await send("Emulation.setDeviceMetricsOverride", { width: 1320, height: 860, deviceScaleFactor: 1, mobile: false }, sessionId);
  await send("Page.addScriptToEvaluateOnNewDocument", { source: `(${installFixture.toString()})()` }, sessionId);
  const button = (text) => evaluate(`(() => {
    const buttons = [...document.querySelectorAll('button')];
    const button = buttons.find((b) => b.textContent.trim() === ${JSON.stringify(text)}) || buttons.find((b) => b.textContent.includes(${JSON.stringify(text)}));
    if (!button || button.disabled) throw new Error('按钮不存在或已禁用：' + ${JSON.stringify(text)});
    button.click();
  })()`);
  const click = (label) => evaluate(`(() => { const e = document.querySelector(${JSON.stringify(`[aria-label="${label}"]`)}); if (!e || e.disabled) throw new Error('控件不可用：' + ${JSON.stringify(label)}); e.click(); })()`);
  const setField = (label, value) => evaluate(`(() => {
    const e = document.querySelector(${JSON.stringify(`[aria-label="${label}"]`)});
    if (!e || e.disabled) throw new Error('字段不可编辑：' + ${JSON.stringify(label)});
    const prototype = e.tagName === 'SELECT' ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, 'value').set.call(e, ${JSON.stringify(value)});
    e.dispatchEvent(new Event(e.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true }));
  })()`);
  const has = (text) => until(() => evaluate(`document.body.textContent.includes(${JSON.stringify(text)})`), text);
  const present = (selector) => until(() => evaluate(`!!document.querySelector(${JSON.stringify(selector)})`), selector);
  const absent = (selector) => until(() => evaluate(`!document.querySelector(${JSON.stringify(selector)})`), `关闭 ${selector}`);
  const latest = (command) => evaluate(`window.testCalls.filter((c) => c.command === ${JSON.stringify(command)}).at(-1)?.args`);
  const count = (command) => evaluate(`window.testCalls.filter((c) => c.command === ${JSON.stringify(command)}).length`);
  const ready = () => until(() => evaluate("document.querySelectorAll('.library-table tbody tr').length >= 3 && !document.querySelector('[aria-label=\"刷新配置与状态\"]').disabled"), '配置库加载');
  const previewReady = () => present('[aria-label="分发变更预览"]');
  const importReady = () => present('[aria-label="选择导入 alpha"]');
  const importPreviewReady = () => present('[aria-label="导入差异预览"]');
  const escape = async () => { await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape' }, sessionId); await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape' }, sessionId); };
  async function screenshot(name) {
    await mkdir('test-artifacts', { recursive: true });
    const { data } = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false }, sessionId);
    await writeFile(`test-artifacts/${name}.png`, Buffer.from(data, 'base64'));
  }
  async function fits() {
    assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth && document.querySelector(".main").scrollWidth <= document.querySelector(".main").clientWidth'), true, '主界面不应横向溢出');
  }
  try {
    await send('Page.navigate', { url: 'http://127.0.0.1:4173/' }, sessionId);
    await ready();
    assert.equal(await evaluate('document.querySelector("h1").textContent'), '我的 MCP');
    assert.equal(await evaluate('document.querySelectorAll("nav button").length'), 3);
    assert.equal(await evaluate('document.querySelector("[aria-label=\"选择项目目录\"]")'), null);
    await screenshot('library'); await fits();
    await click('查看 project-only');
    assert.equal(await evaluate('document.querySelector("[aria-label=\"project-only → Claude Code\"]").disabled'), true);
    await has('仅适用于项目配置');
    await click('查看 alpha'); await click('alpha → Claude Code');
    await click('查看 beta'); await click('beta → Codex CLI');
    await evaluate('document.querySelector(".other-agents summary").click()');
    assert.equal(await evaluate('document.querySelectorAll(".agent-target input").length'), 14);
    assert.equal(await evaluate('document.querySelector(".selection-bar").getBoundingClientRect().bottom <= innerHeight'), true);
    await screenshot('library-inspector');
    await button('预览变更'); await previewReady();
    const expected = { 'claude-code': ['alpha'], codex: ['beta'] };
    assert.deepEqual((await latest('plan')).selection, expected);
    assert.equal((await latest('plan')).projectRoot, null);
    assert.equal(await evaluate('document.querySelectorAll(".actual-file-path").length'), 2);
    await screenshot('distribution-preview');
    await evaluate('document.querySelector(".preview-options").open = true');
    await setField('预览写入策略', 'replace'); await has('需要重新预览');
    assert.equal(await evaluate('[...document.querySelectorAll("button")].some((b) => b.textContent.includes("确认写入"))'), false);
    await button('重新生成预览'); await present('.file-plan');
    const planned = await latest('plan'); assert.equal(planned.mode, 'replace');
    await button('确认写入 2 个文件'); await has('配置已写入');
    assert.deepEqual(await latest('apply'), { ...planned, dryRun: false });
    await screenshot('history');
    await evaluate('window.confirmAnswer = false'); await button('恢复此备份'); assert.equal(await count('restore'), 0);
    await evaluate('window.confirmAnswer = true'); await button('恢复此备份'); await has('已恢复 1 个文件');

    // 勾选变动后，迟到的响应不能恢复已经失效的预览。
    await click('我的 MCP'); await ready(); await click('查看 alpha'); await click('alpha → Claude Code');
    await evaluate('window.delayPlan = true'); await button('预览变更');
    await until(() => evaluate('typeof window.releasePlan === "function"'), '延迟计划');
    await click('alpha → Claude Code'); await evaluate('window.delayPlan = false; window.releasePlan()'); await pause(180);
    assert.equal(await evaluate('document.querySelector("dialog")'), null);
    await button('批量矩阵'); await present('.matrix');
    await evaluate('document.querySelector("button[title=\"alpha → Claude Code\"]").click()');
    await escape(); await absent('dialog');
    assert.equal(await evaluate('document.querySelector("[aria-label=\"alpha → Claude Code\"]").checked'), true);
    await click('alpha → Claude Code');

    // 编辑采用数组和键值字段，不能用逗号拆分输入；未展示的高级字段原样保留。
    await button('编辑配置'); await present('[aria-label="MCP 名称"]');
    await setField('MCP 描述', '更新后的工具说明');
    await setField('启动参数 1', 'a,b=c');
    await evaluate('document.querySelector(".key-values .text-button").click()');
    await setField('环境变量名称 1', 'CUSTOM_VALUE'); await setField('环境变量值 1', '  one,two=value  ');
    assert.equal(await evaluate('document.querySelector("[aria-label=\"环境变量值 1\"]").type'), 'password');
    await screenshot('entry-editor');
    await button('保存到 MCP 库'); await absent('dialog');
    const saved = (await latest('save_catalog')).catalog.entries.find((e) => e.name === 'alpha');
    assert.deepEqual(saved.args, ['a,b=c', '  keep spaces  ']);
    assert.equal(saved.env.CUSTOM_VALUE, '  one,two=value  ');
    assert.equal(saved.enabled, false); assert.equal(saved.timeout_seconds, 45);
    assert.deepEqual(saved.overrides, { windows: { command: 'python.exe' } });
    assert.equal(await count('apply'), 1, '保存库不能自动分发');
    await button('添加 MCP'); await present('[aria-label="MCP 名称"]');
    await setField('MCP 名称', 'alpha'); await button('保存到 MCP 库'); await has('配置库已有同名 MCP');
    const savesBefore = await count('save_catalog');
    await evaluate('window.confirmAnswer = false'); await escape();
    assert.equal(await evaluate('!!document.querySelector("dialog[open]")'), true);
    assert.match(await evaluate('window.lastConfirmation'), /未保存/);
    for (let i = 0; i < 14; i++) {
      await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Tab', code: 'Tab' }, sessionId);
      await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Tab', code: 'Tab' }, sessionId);
      assert.equal(await evaluate('!!document.activeElement.closest("dialog") || document.activeElement === document.body'), true, '焦点不能进入背景控件');
    }
    await evaluate('window.confirmAnswer = true'); await escape(); await absent('dialog');
    assert.equal(await count('save_catalog'), savesBefore);

    // 项目范围只改变可分发状态，不隐藏配置库条目；目录取消不写设置。
    await setField('配置范围', 'project'); await present('[aria-label="项目目录"]');
    const settingsBefore = await count('save_settings');
    await evaluate('window.cancelDirectory = true'); await button('选择目录…');
    await until(() => evaluate('!document.querySelector("[aria-label=\"项目目录\"]").disabled'), '取消目录选择');
    assert.equal(await count('save_settings'), settingsBefore);
    await evaluate('window.cancelDirectory = false'); await button('选择目录…'); await absent('dialog'); await ready();
    assert.equal((await latest('matrix')).projectRoot, 'C:/test/中文项目');
    assert.equal(await evaluate('document.querySelectorAll(".library-table tbody tr").length'), 3);
    await click('查看 project-only');
    assert.equal(await evaluate('document.querySelector("[aria-label=\"project-only → Claude Code\"]").disabled'), false);
    await click('project-only → Claude Code'); await button('预览变更'); await previewReady();
    assert.equal((await latest('plan')).projectRoot, 'C:/test/中文项目');
    await button('返回修改'); await absent('dialog');
    await setField('配置范围', 'user'); await ready();
    assert.equal((await latest('detect')).projectRoot, null);
    assert.equal(await evaluate('document.querySelector("[aria-label=\"选择项目目录\"]")'), null);

    // Agent 读取与导入保持独立；晚到的旧来源读取不覆盖新选择。
    await click('Agents'); await click('查看 Codex CLI 配置'); await present('[aria-label="已有 MCP 配置"]');
    await evaluate('document.querySelector(".existing-entry").open = true');
    assert.equal(await evaluate('document.querySelector(".existing-entry pre").textContent.includes("[已隐藏]")'), true);
    await screenshot('agents');
    await evaluate('window.delayRead = true'); await click('查看 Claude Code 配置');
    await until(() => evaluate('typeof window.releaseRead === "function"'), '延迟读取');
    await evaluate('window.delayRead = false'); await click('查看 Codex CLI 配置'); await present('[aria-label="已有 MCP 配置"]');
    await evaluate('window.releaseRead()'); await pause(150);
    assert.equal(await evaluate('document.querySelector(".existing-config h2").textContent'), 'codex');
    await evaluate('window.failRead = true'); await click('查看 Claude Code 配置'); await has('配置解析失败');
    assert.equal(await evaluate('document.querySelector(".existing-config")'), null);
    await evaluate('window.failRead = false'); await click('查看 Codex CLI 配置'); await present('.existing-config');
    await button('导入到 MCP 库'); await importReady();
    assert.equal(await evaluate('document.querySelector("[aria-label=\"选择导入 unsupported\"]").disabled'), true);
    await button('取消导入'); await absent('dialog'); assert.equal(await count('commit_import'), 0);
    await button('导入到 MCP 库'); await importReady(); await click('选择导入 alpha');
    assert.equal(await evaluate('document.querySelector("[aria-label=\"处理方式 alpha\"]").value'), 'skip');
    await setField('处理方式 alpha', 'rename'); await setField('目标名称 alpha', 'alpha-imported');
    await setField('环境变量 alpha /auth/token_env', 'API_KEY'); await button('预览导入'); await importPreviewReady();
    assert.deepEqual((await latest('preview_import')).choices, [{ sourceName: 'alpha', action: 'rename', targetName: 'alpha-imported', bindings: { '/auth/token_env': 'API_KEY' } }]);
    await setField('目标名称 alpha', 'alpha-changed'); await absent('[aria-label="导入差异预览"]');
    await setField('处理方式 alpha', 'replace'); await button('预览导入'); await importPreviewReady();
    assert.equal(await evaluate('[...document.querySelectorAll("button")].find((b) => b.textContent === "确认导入配置库").disabled'), true);
    await evaluate('document.querySelector(".import-preview input[type=checkbox]").click(); window.staleImport = true');
    await button('确认导入配置库'); await has('源配置已变化'); await absent('[aria-label="导入差异预览"]');
    await button('取消导入'); await absent('dialog'); await evaluate('window.staleImport = false');
    await button('导入到 MCP 库'); await importReady(); await click('选择导入 alpha'); await setField('处理方式 alpha', 'rename');
    await setField('目标名称 alpha', 'alpha-imported'); await setField('环境变量 alpha /auth/token_env', 'API_KEY');
    await click('选择导入 new-mcp'); await setField('环境变量 new-mcp /args/0', 'MCP_ARGUMENT');
    await button('预览导入'); await importPreviewReady(); await screenshot('agent-import-preview');
    await button('确认导入配置库'); await has('已导入 2 项到 MCP 库'); await absent('dialog');
    assert.equal(await evaluate('document.querySelector("h1").textContent'), '我的 MCP');
    assert.deepEqual(Object.keys(await latest('commit_import')).sort(), ['previewId', 'projectRoot', 'scope']);
    assert.equal(await evaluate('!!document.querySelector("[aria-label=\"查看 alpha-imported\"]")'), true);
    assert.equal(await count('save_catalog'), savesBefore, '导入不走编辑器的明文保存接口');

    // 部分写入、回滚失败和审计失败不能展示成全部成功。
    await click('查看 beta'); await click('beta → Codex CLI'); await evaluate('window.applyFailure = true');
    await button('预览变更'); await previewReady(); await button('确认写入 1 个文件'); await has('自动回滚失败，请检查备份');
    await has('审计写入失败，文件未回滚'); await has('部分目标需要处理');

    // 默认库不存在是正常引导，损坏文件则禁止空白覆盖。
    await click('我的 MCP'); await evaluate('window.catalogMissing = true'); await click('刷新配置与状态'); await has('把你的第一个 MCP 带进来');
    assert.equal(await evaluate('document.querySelector(".error-state")'), null); await screenshot('first-use');
    await button('手动添加'); await setField('MCP 名称', 'first-mcp'); await button('保存到 MCP 库'); await absent('dialog');
    assert.equal(await count('create_catalog'), 1); await has('first-mcp');
    await evaluate('window.catalogCorrupt = true'); await click('刷新配置与状态'); await has('暂时无法打开配置库');
    assert.equal(await evaluate('[...document.querySelectorAll("button")].find((b) => b.textContent === "添加 MCP").disabled'), true);
    const corruptSaves = await count('save_catalog');
    await button('检查配置库位置'); await present('[aria-label="配置库文件路径"]');
    await setField('配置库文件路径', 'C:/test/another.yaml'); await button('使用此配置库'); await has('配置库格式无效');
    assert.equal(await count('save_catalog'), corruptSaves); await button('取消'); await absent('dialog');
    await evaluate('window.catalogCorrupt = false'); await click('刷新配置与状态'); await has('first-mcp');

    // 软件默认设置下次启动使用，当前上下文不被偷偷切换。
    await click('软件设置'); await setField('启动默认范围', 'project'); await setField('默认写入策略', 'replace');
    await setField('备份目录', 'C:/test/new-backups'); await evaluate('window.rejectSettings = true');
    await button('保存全局设置'); await has('测试设置保存失败');
    assert.equal(await evaluate('JSON.parse(sessionStorage.getItem("fixtureSettings")).defaultScope'), 'user');
    await evaluate('window.rejectSettings = false'); await button('保存全局设置'); await has('设置已保存。默认配置库');
    await screenshot('software-settings');
    await click('我的 MCP'); assert.equal(await evaluate('document.querySelector("[aria-label=\"配置范围\"]").value'), 'user');
    await send('Page.reload', {}, sessionId); await ready();
    assert.equal(await evaluate('document.querySelector("[aria-label=\"配置范围\"]").value'), 'project');
    assert.equal((await latest('matrix')).projectRoot, 'C:/test/中文项目');
    await click('查看 beta'); await evaluate('document.querySelector(".technical-details").open = true');
    assert.equal(await evaluate('document.querySelector("[aria-label=\"写入策略\"]").value'), 'replace');
    await click('软件设置'); assert.equal(await evaluate('document.querySelector("[aria-label=\"备份目录\"]").value'), 'C:/test/new-backups');
    await click('我的 MCP');
    await send('Emulation.setDeviceMetricsOverride', { width: 1040, height: 680, deviceScaleFactor: 1, mobile: false }, sessionId);
    await click('查看 beta'); await click('beta → Codex CLI'); await fits();
    assert.equal(await evaluate('document.querySelector(".selection-bar").getBoundingClientRect().bottom <= innerHeight'), true);
    await screenshot('library-small-desktop');
    await send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: false }, sessionId);
    await fits(); await screenshot('library-narrow');
    await button('编辑配置'); await present('dialog');
    assert.equal(await evaluate('document.querySelector("dialog").getBoundingClientRect().right <= innerWidth'), true);
    await screenshot('editor-narrow');
  } catch (error) {
    await screenshot('failure').catch(() => {});
    console.error(await evaluate('document.body.innerText').catch(() => '页面无法读取'));
    throw error;
  }
});
