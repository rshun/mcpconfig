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
test("真实 GUI 验证选择写入和只读查看已有 Agent MCP", { timeout: 90000 }, async (t) => {
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
          case "create_catalog": return [];
          case "app_info": return { version: "test", platform: "windows", dataDir: "test-only" };
          case "detect": return activeClients.map((c) => ({ ...c, state: "ready", configured: 1, path: "test-only",
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
            rows: ["alpha", "beta"].map((name) => ({
              name, transport: "stdio", scope: args.scope,
              cells: activeClients.map(({ id: clientId }) => ({ clientId, state: "absent", detail: null })),
            })),
            };
          }
          case "load_catalog": return { catalog: { version: 1, entries: ["alpha", "beta"].map((name) => ({name, transport: "stdio", command: "python"})) }, issues: [], entryCount: 2 };
          case "backups": return [];
          case "plan": {
            if (window.delayPlan) await new Promise((resolve) => { window.releasePlan = resolve; });
            const files = Object.entries(args.selection).map(([clientId, names]) => ({
              clientId, clientName: clientId, path: "test-only", format: "JSON", scope: args.scope,
              existed: false, blocked: null, newCount: names.length, updateCount: 0, unchangedCount: 0,
              diff: `+${names.join(",")}`, warnings: [], willWrite: true,
            }));
            return { entryCount: 2, files, writable: files.length, issues: [], missingVars: [], skipped: [] };
          }
          case "apply": return window.applyFailure ? {
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
  await send("Page.navigate", { url: "http://127.0.0.1:4173/?page=matrix" }, sessionId);
  await until(() => evaluate("document.querySelectorAll('.cell-btn').length === 4"), "矩阵加载");
  const button = (text) => evaluate(`(() => {
    const button = [...document.querySelectorAll('button')].find((b) => b.textContent.includes(${JSON.stringify(text)}));
    if (!button || button.disabled) throw new Error('按钮不存在或已禁用');
    button.click();
  })()`);
  const cell = (title) => evaluate(`document.querySelector(${JSON.stringify(`button[title="${title}"]`)}).click()`);
  const waitDiff = () => until(() => evaluate("document.querySelector('h1')?.textContent === '差异预览'"), "差异预览");
  const lastPlan = () => evaluate("window.testCalls.filter((c) => c.command === 'plan').at(-1).args");
  async function screenshot(name) {
    await mkdir("test-artifacts", { recursive: true });
    const { data } = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false }, sessionId);
    await writeFile(`test-artifacts/${name}.png`, Buffer.from(data, "base64"));
  }
  const expected = { "claude-code": ["alpha"], codex: ["beta"] };

  await cell("alpha → Claude Code");
  await cell("beta → Codex CLI");
  await button("生成差异预览");
  await waitDiff();
  assert.deepEqual((await lastPlan()).selection, expected);

  await button("客户端配置");
  await cell("alpha → Claude Code");
  await button("差异预览");
  await until(() => evaluate("document.body.textContent.includes('还没有生成计划')"), "修改勾选后旧预览失效");
  assert.equal(await evaluate("[...document.querySelectorAll('button')].some((b) => b.textContent.includes('确认写入'))"), false);

  await button("客户端配置");
  await cell("alpha → Claude Code");
  await evaluate("window.delayPlan = true");
  await button("生成差异预览");
  await until(() => evaluate("typeof window.releasePlan === 'function'"), "延迟的预览请求");
  await cell("alpha → Claude Code");
  await evaluate("window.delayPlan = false; window.releasePlan()");
  await until(() => evaluate("!document.querySelector('.bottombar button').disabled"), "预览请求结束");
  await button("差异预览");
  assert.equal(await evaluate("document.body.textContent.includes('还没有生成计划')"), true, "迟到的响应不能恢复旧预览");

  await button("客户端配置");
  await cell("alpha → Claude Code");
  await button("生成差异预览");
  await waitDiff();
  await evaluate("const mode = document.querySelectorAll('.topbar select')[1]; mode.value = 'replace'; mode.dispatchEvent(new Event('change', { bubbles: true }))");
  await until(() => evaluate("document.body.textContent.includes('还没有生成计划')"), "覆盖模式变更后旧预览失效");
  await button("客户端配置");
  await button("生成差异预览");
  await waitDiff();
  const planned = await lastPlan();
  assert.deepEqual(planned.selection, expected);
  assert.equal(planned.mode, "replace");
  await button("确认写入");
  await until(() => evaluate("window.testCalls.some((c) => c.command === 'apply')"), "写入 IPC");
  const applied = await evaluate("window.testCalls.find((c) => c.command === 'apply').args");
  assert.deepEqual(applied, { ...planned, dryRun: false });

  await until(() => evaluate("document.querySelector('h1')?.textContent === '备份与回滚'"), "应用流程结束");
  const mutations = await evaluate("window.testCalls.filter((c) => ['apply','save_catalog','restore'].includes(c.command)).length");
  await button("Agent");
  // 按行选择客户端，避免同名操作按钮被错误定位。
  const readClient = (index) => evaluate(`document.querySelectorAll('table.grid tbody tr')[${index}].querySelector('button').click()`);
  await readClient(1);
  await until(() => evaluate("document.querySelector('[aria-label=\"已有 MCP 配置\"]')?.textContent.includes('codex-已有服务')"), "读取 Codex 已有条目");
  await evaluate("document.querySelector('[aria-label=\"已有 MCP 配置\"] summary').click()");
  assert.equal(await evaluate("document.querySelector('[aria-label=\"已有 MCP 配置\"] details').open"), true);
  assert.equal(await evaluate("document.querySelector('[aria-label=\"已有 MCP 配置\"] pre').textContent.includes('[已隐藏]')"), true);
  const readArgs = await evaluate("window.testCalls.filter((c) => c.command === 'read_client_config').at(-1).args");
  assert.deepEqual(readArgs, { clientId: "codex", scope: "user", projectRoot: null });

  await evaluate("window.delayRead = true");
  await readClient(0);
  await until(() => evaluate("typeof window.releaseRead === 'function'"), "延迟读取");
  await evaluate("window.delayRead = false");
  await readClient(1);
  await until(() => evaluate("document.querySelector('[aria-label=\"已有 MCP 配置\"]')?.textContent.includes('codex-已有服务')"), "后一次读取完成");
  await evaluate("window.releaseRead()");
  await pause(150);
  assert.equal(await evaluate("document.querySelector('[aria-label=\"已有 MCP 配置\"]')?.textContent.includes('claude-code-已有服务')"), false);

  await evaluate("window.failRead = true");
  await readClient(0);
  await until(() => evaluate("document.body.textContent.includes('配置解析失败')"), "读取错误提示");
  assert.equal(await evaluate("document.querySelector('[aria-label=\"已有 MCP 配置\"]') === null"), true);
  await evaluate("window.failRead = false; window.missingConfig = true");
  await readClient(0);
  await until(() => evaluate("document.body.textContent.includes('配置文件尚不存在')"), "缺失配置状态");

  await evaluate("window.missingConfig = false; window.delayRead = true; window.releaseRead = null");
  await readClient(0);
  await until(() => evaluate("typeof window.releaseRead === 'function'"), "作用域切换前的读取");
  await evaluate("const scope = document.querySelectorAll('.topbar select')[0]; scope.value = 'project'; scope.dispatchEvent(new Event('change', { bubbles: true }))");
  await evaluate("window.delayRead = false; window.releaseRead()");
  await pause(150);
  assert.equal(await evaluate("document.querySelector('[aria-label=\"已有 MCP 配置\"]') === null"), true);

  assert.equal(await evaluate("document.querySelectorAll('[aria-label=\"项目目录选择\"]').length"), 1);
  await button("选择目录…");
  await until(() => evaluate("window.testCalls.some((c) => c.command === 'detect' && c.args.projectRoot === 'C:/test/中文项目') && !document.querySelector('[aria-label=\"项目目录\"]').disabled"), "项目选择生效");
  await evaluate("window.failMatrix = true");
  await button("加载");
  await until(() => evaluate("document.body.textContent.includes('清单不存在（测试）')"), "清单加载失败");
  await readClient(1);
  await until(() => evaluate("document.querySelector('[aria-label=\"已有 MCP 配置\"]')?.textContent.includes('codex-已有服务')"), "没有清单仍能读取已有配置");
  assert.equal(await evaluate("window.testCalls.filter((c) => ['apply','save_catalog','restore'].includes(c.command)).length"), mutations);

  await evaluate("window.failMatrix = false; window.applyFailure = true");
  await button("刷新");
  await button("客户端配置");
  await until(() => evaluate("document.querySelectorAll('.cell-btn').length === 4"), "失败回执测试矩阵");
  await cell("alpha → Claude Code");
  await button("生成差异预览");
  await waitDiff();
  await button("确认写入");
  await until(() => evaluate("document.body.textContent.includes('自动回滚失败，请检查备份')"), "回滚失败明确提示");
  assert.equal(await evaluate("document.body.textContent.includes('审计写入失败，文件未回滚')"), true);
  assert.equal(await evaluate("document.body.textContent.includes('已自动回滚')"), false);

  // 工作台的两种视角共用逐对勾选，不能扩展成交叉组合。
  await button("配置工作台");
  await until(() => evaluate("!!document.querySelector('[aria-label=\"alpha → Claude Code\"]')"), "工作台加载");
  await evaluate("document.querySelector('[aria-label=\"alpha → Claude Code\"]').click()");
  await evaluate("[...document.querySelectorAll('.work-item')].find((b) => b.textContent.includes('beta')).click()");
  await evaluate("document.querySelector('[aria-label=\"beta → Codex CLI\"]').click()");
  await screenshot("workbench-project");
  await button("按 Agent 查看");
  await screenshot("workbench-agent");
  await button("预览变更");
  await waitDiff();
  assert.deepEqual((await lastPlan()).selection, expected);
  assert.equal((await lastPlan()).projectRoot, "C:/test/中文项目");
  assert.equal((await lastPlan()).scope, "project");

  // 取消目录选择不得保存设置或改变已预览内容。
  await evaluate("window.cancelDirectory = true");
  const settingsSaves = await evaluate("window.testCalls.filter((c) => c.command === 'save_settings').length");
  await button("选择目录…");
  await until(() => evaluate("!document.querySelector('[aria-label=\"项目目录\"]').disabled"), "取消目录选择结束");
  assert.equal(await evaluate("window.testCalls.filter((c) => c.command === 'save_settings').length"), settingsSaves);
  assert.equal(await evaluate("document.querySelector('h1')?.textContent"), "差异预览");
  await evaluate("const currentScopeControl = document.querySelector('[aria-label=\"配置范围\"]'); currentScopeControl.value = 'user'; currentScopeControl.dispatchEvent(new Event('change', { bubbles: true }))");
  await until(() => evaluate("document.querySelector('[aria-label=\"项目目录选择\"]') === null"), "全局范围隐藏目录");
  await until(() => evaluate("window.testCalls.filter((c) => c.command === 'detect').at(-1).args.scope === 'user'"), "全局检测刷新");
  assert.equal(await evaluate("window.testCalls.filter((c) => c.command === 'detect').at(-1).args.projectRoot"), null);
  assert.equal(await evaluate("document.body.textContent.includes('还没有生成计划')"), true);

  // 软件设置持久化与当前工作范围相互独立；重启使用保存的默认值。
  await button("全局设置");
  await evaluate("const startupScopeControl = document.querySelector('[aria-label=\"启动默认范围\"]'); startupScopeControl.value = 'project'; startupScopeControl.dispatchEvent(new Event('change', { bubbles: true }))");
  await evaluate("const defaultStrategy = document.querySelector('[aria-label=\"默认写入策略\"]'); defaultStrategy.value = 'replace'; defaultStrategy.dispatchEvent(new Event('change', { bubbles: true }))");
  await evaluate("const input = document.querySelector('[aria-label=\"备份目录\"]'); Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'C:/test/new-backups'); input.dispatchEvent(new Event('input', { bubbles: true }))");
  await evaluate("window.rejectSettings = true");
  await button("保存全局设置");
  await until(() => evaluate("document.body.textContent.includes('测试设置保存失败')"), "设置失败提示");
  assert.equal(await evaluate("JSON.parse(sessionStorage.getItem('fixtureSettings')).defaultScope"), "user");
  await evaluate("window.rejectSettings = false");
  await button("保存全局设置");
  await until(() => evaluate("document.body.textContent.includes('设置已保存。默认配置库')"), "设置保存完成");
  assert.equal(await evaluate("document.querySelector('[aria-label=\"配置范围\"]').value"), "user");
  await send("Page.reload", {}, sessionId);
  await until(() => evaluate("document.querySelector('[aria-label=\"项目目录\"]')?.value === 'C:/test/中文项目'"), "重启恢复项目范围及目录");
  assert.equal(await evaluate("document.querySelector('[aria-label=\"写入策略\"]').value"), "replace");
  const restored = await evaluate("window.testCalls.find((c) => c.command === 'matrix').args");
  assert.equal(restored.projectRoot, "C:/test/中文项目");
  await button("全局设置");
  assert.equal(await evaluate("document.querySelector('[aria-label=\"备份目录\"]').value"), "C:/test/new-backups");
  await screenshot("global-settings");
  await evaluate("window.manyClients = true");
  await button("配置工作台");
  await button("刷新");
  await until(() => evaluate("document.querySelectorAll('.work-targets input').length === 14"), "14 个目标 Agent");
  assert.equal(await evaluate("document.querySelector('.work-targets').scrollHeight > document.querySelector('.work-targets').clientHeight"), true);
  assert.equal(await evaluate("document.querySelector('.bottombar').getBoundingClientRect().bottom <= innerHeight"), true, "完整目标列表不能挤出预览操作");
  await screenshot("workbench-fourteen-agents");
});
