import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp } from "node:fs/promises";
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
test("真实 GUI 保留配对、使旧预览失效，并按预览选择发出写入请求", { timeout: 60000 }, async (t) => {
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
  ], { stdio: ["ignore", "ignore", "ignore", "pipe", "pipe"] });
  t.after(() => chrome.kill());
  await once(chrome, "spawn");
  let sequence = 0;
  let buffer = "";
  const pending = new Map();
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
        reject(new Error(`CDP 超时：${method}`));
      }, 10000);
      pending.set(id, { resolve, reject, timer });
      chrome.stdio[3].write(JSON.stringify({ id, method, params, sessionId }) + "\0");
    });
  }
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
        window.testCalls.push({ command, args: structuredClone(args) });
        switch (command) {
          case "app_info": return { version: "test", platform: "windows", dataDir: "test-only" };
          case "detect": return clients.map((c) => ({ ...c, state: "ready" }));
          case "matrix": return {
            platform: "windows", scope: args.scope, clients, issues: [],
            rows: ["alpha", "beta"].map((name) => ({
              name, transport: "stdio", scope: args.scope,
              cells: ids.map((clientId) => ({ clientId, state: "absent", detail: null })),
            })),
          };
          case "load_catalog": return { catalog: { version: 1, entries: [] }, issues: [], entryCount: 0 };
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
          case "apply": return { written: 2, blocked: 0, failed: 0, backupId: null, outcomes: [] };
          default: throw new Error(`未预期的 IPC：${command}`);
        }
      },
    };
  }
  await send("Page.enable", {}, sessionId);
  await send("Page.addScriptToEvaluateOnNewDocument", { source: `(${installFixture.toString()})()` }, sessionId);
  await send("Page.navigate", { url: "http://127.0.0.1:4173" }, sessionId);
  await until(() => evaluate("document.querySelectorAll('.cell-btn').length === 4"), "矩阵加载");
  const button = (text) => evaluate(`(() => {
    const button = [...document.querySelectorAll('button')].find((b) => b.textContent.includes(${JSON.stringify(text)}));
    if (!button || button.disabled) throw new Error('按钮不存在或已禁用');
    button.click();
  })()`);
  const cell = (title) => evaluate(`document.querySelector(${JSON.stringify(`button[title="${title}"]`)}).click()`);
  const waitDiff = () => until(() => evaluate("document.querySelector('h1')?.textContent === '差异预览'"), "差异预览");
  const lastPlan = () => evaluate("window.testCalls.filter((c) => c.command === 'plan').at(-1).args");
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
});
