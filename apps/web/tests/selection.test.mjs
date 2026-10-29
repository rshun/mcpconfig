import assert from "node:assert/strict";
import { extname } from "node:path";
import { registerHooks } from "node:module";
import { test } from "node:test";
import { mockIPC } from "@tauri-apps/api/mocks";

// Node 24 原生擦除 TS 类型；仅补上 Vite 使用的相对路径扩展名解析。
const sourceRoot = new URL("../src/", import.meta.url).href;
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (context.parentURL?.startsWith(sourceRoot) && specifier.startsWith(".") && !extname(specifier)) {
      return nextResolve(`${specifier}.ts`, context);
    }
    return nextResolve(specifier, context);
  },
});

globalThis.window = { location: { search: "" }, crypto: globalThis.crypto };
const calls = [];
mockIPC((command, args) => {
  calls.push({ command, args: structuredClone(args) });
  return {};
});
const { api } = await import("../src/api.ts");
const { selectionFromCells } = await import("../src/selection.ts");
const { mockCall } = await import("../src/mock.ts");

test("不同客户端的单元格选择完整传入预览和写入 IPC，不扩展成行列组合", async () => {
  const request = {
    catalogPath: "catalog.yaml",
    selection: selectionFromCells(new Set(["filesystem::claude-code", "context7::codex"])),
    scope: "user",
    mode: "merge",
    allowCommentStrip: false,
  };
  calls.length = 0;
  await api.plan(request);
  await api.apply(request, false);
  assert.deepEqual(request.selection, { "claude-code": ["filesystem"], codex: ["context7"] });
  assert.deepEqual(calls, [
    { command: "plan", args: request },
    { command: "apply", args: { ...request, dryRun: false } },
  ]);
});

test("空选择在预览与写入 IPC 中保持为空", async () => {
  const request = {
    catalogPath: "catalog.yaml", selection: selectionFromCells([]),
    scope: "user", mode: "replace", allowCommentStrip: false,
  };
  calls.length = 0;
  await api.plan(request);
  await api.apply(request, true);
  assert.deepEqual(calls.map(({ args }) => args.selection), [{}, {}]);
  assert.equal(calls[1].args.dryRun, true);
});

test("名称中的分隔符不会导致 MCP 名称被截断，同客户端可选择多项", () => {
  assert.deepEqual(selectionFromCells(["team::search::codex", "fs::codex", "fs::codex"]), {
    codex: ["team::search", "fs"],
  });
  for (const invalid of ["fs", "::codex", "fs::"]) {
    assert.throws(() => selectionFromCells([invalid]), /无效/);
  }
});

test("浏览器演示的差异与写入也只涉及逐客户端选中的 MCP", async () => {
  const args = { scope: "user", selection: { "claude-code": ["filesystem"], codex: ["context7"] } };
  const plan = await mockCall("plan", args);
  assert.equal(plan.entryCount, 2);
  assert.equal(plan.files.length, 2);
  const claude = plan.files.find((file) => file.clientId === "claude-code");
  const codex = plan.files.find((file) => file.clientId === "codex");
  assert.match(claude.diff, /filesystem/);
  assert.doesNotMatch(claude.diff, /context7|stockdb/);
  assert.match(codex.diff, /context7/);
  assert.doesNotMatch(codex.diff, /filesystem|stockdb/);
  const receipt = await mockCall("apply", args);
  assert.equal(receipt.written, plan.writable);
  assert.deepEqual(receipt.outcomes.map((file) => file.clientId), plan.files.map((file) => file.clientId));
});

test("浏览器演示的空映射和空条目列表均不生成文件或备份", async () => {
  for (const selection of [{}, { codex: [] }]) {
    const args = { scope: "user", selection };
    const plan = await mockCall("plan", args);
    assert.deepEqual(plan.files, []);
    const receipt = await mockCall("apply", args);
    assert.equal(receipt.written, 0);
    assert.equal(receipt.backupId, null);
    assert.deepEqual(receipt.outcomes, []);
  }
});

test("浏览器演示拒绝缺失、未知或作用域错误的选择", async () => {
  for (const args of [
    {},
    { selection: { unknown: ["filesystem"] } },
    { selection: { codex: ["missing"] } },
    { scope: "user", selection: { codex: ["github"] } },
  ]) {
    await assert.rejects(mockCall("plan", args));
    await assert.rejects(mockCall("apply", args));
  }
});

test("读取已有 Agent MCP 的 IPC 不需要清单路径，也不触发写入", async () => {
  calls.length = 0;
  await api.readClientConfig("codex", "user");
  assert.deepEqual(calls, [{ command: "read_client_config", args: { clientId: "codex", scope: "user", projectRoot: null } }]);
  const shown = await mockCall("read_client_config", { clientId: "codex", scope: "user" });
  assert.equal(shown.clientId, "codex");
  assert.equal(shown.entries[0].config.env.API_KEY, "[已隐藏]");
  await assert.rejects(mockCall("read_client_config", { clientId: "unknown", scope: "user" }));
  await assert.rejects(mockCall("read_client_config", { clientId: "codex", scope: "local" }));
});

test("项目目录完整传入检测、矩阵和只读查看，设置独立保存", async () => {
  calls.length = 0;
  const projectRoot = "C:/test/中文项目";
  await api.detect("project", projectRoot);
  await api.matrix("C:/test/catalog.yaml", "project", projectRoot);
  await api.readClientConfig("codex", "project", projectRoot);
  assert.deepEqual(calls.map((call) => call.args.projectRoot), [projectRoot, projectRoot, projectRoot]);
  assert.deepEqual(calls.map((call) => call.args.scope), ["project", "project", "project"]);
  const settings = { version: 1, catalogPath: "C:/test/catalog.yaml", backupDir: "C:/test/backups", defaultScope: "user", defaultMode: "merge", lastProjectDir: projectRoot };
  await api.saveSettings(settings);
  assert.deepEqual(calls.at(-1), { command: "save_settings", args: { settings } });
});

test("导入 IPC 保留上下文、冲突策略和映射，确认只发送后端预览 ID", async () => {
  calls.length = 0;
  const choices = [{ sourceName: "same", action: "rename", targetName: "same-copy", bindings: { "/auth/token_env": "SERVICE_AUTH" } }];
  await api.scanImport("C:/test/catalog.yaml", "codex", "project", "C:/test/project");
  await api.previewImport("scan-id", choices, "project", "C:/test/project");
  await api.commitImport("preview-id", "project", "C:/test/project");
  await api.cancelImport("scan-id");
  assert.deepEqual(calls, [
    { command: "scan_import", args: { catalogPath: "C:/test/catalog.yaml", clientId: "codex", scope: "project", projectRoot: "C:/test/project" } },
    { command: "preview_import", args: { scanId: "scan-id", choices, scope: "project", projectRoot: "C:/test/project" } },
    { command: "commit_import", args: { previewId: "preview-id", scope: "project", projectRoot: "C:/test/project" } },
    { command: "cancel_import", args: { scanId: "scan-id" } },
  ]);
});

test("演示导入需要完整变量映射，取消和重复确认均被阻止", async () => {
  const scan = await mockCall("scan_import", { catalogPath: "C:/demo/catalog.yaml", clientId: "codex", scope: "user" });
  const choice = { sourceName: "example-mcp", action: "add", targetName: "example-mcp", bindings: {} };
  await assert.rejects(mockCall("preview_import", { scanId: scan.scanId, choices: [choice] }));
  choice.bindings = { "/args/0": "MCP_ARG", "/env/API_KEY": "MCP_KEY" };
  const preview = await mockCall("preview_import", { scanId: scan.scanId, choices: [choice], scope: "user" });
  const result = await mockCall("commit_import", { previewId: preview.previewId });
  assert.equal(result.imported, 1);
  await assert.rejects(mockCall("commit_import", { previewId: preview.previewId }));
  const cancelled = await mockCall("scan_import", { catalogPath: "C:/demo/catalog.yaml", clientId: "codex", scope: "user" });
  await mockCall("cancel_import", { scanId: cancelled.scanId });
  await assert.rejects(mockCall("preview_import", { scanId: cancelled.scanId, choices: [choice] }));
});
