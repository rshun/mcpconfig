/**
 * 浏览器预览用的 mock 数据。
 *
 * 只在没有 Tauri 运行时（例如 `npm run dev` 用浏览器打开）启用，
 * 目的是让 UI 可以独立评审。桌面端永远走真实 IPC。
 *
 * 打开 mock：URL 加 `?mock=1`
 */
import type { GuiSettings, ImportChoice, ImportPreview, ImportScan, McpSelection, PlanDto, Scope } from "./types";

let settings: GuiSettings = { version: 1, catalogPath: "C:/Users/Demo/.mcp-hub/catalog.yaml", backupDir: "C:/Users/Demo/.mcp-hub/backups", defaultScope: "user", defaultMode: "merge", lastProjectDir: null };
let importSerial = 0;
const importScans = new Map<string, ImportScan>();
const importPreviews = new Map<string, ImportPreview & { scope: Scope; scanId: string }>();

const params =
  typeof window !== "undefined"
    ? new URLSearchParams(window.location.search)
    : new URLSearchParams();

export const mockMode = params.get("mock") === "1";

interface Style {
  root: string[];
  bearer: "native" | "dollar" | "brace" | "dollarEnv" | "literal";
  cmdArray?: boolean;
  urlKey?: string;
  scopes: string[];
}

const STYLES: Record<string, Style> = {
  "claude-code": { root: ["mcpServers"], bearer: "dollar", scopes: ["user", "project", "local"] },
  codex: { root: ["mcp_servers"], bearer: "native", scopes: ["user", "project"] },
  workbuddy: { root: ["mcpServers"], bearer: "literal", scopes: ["user"] },
  "kimi-code": { root: ["mcpServers"], bearer: "literal", scopes: ["user"] },
  opencode: {
    root: ["mcp", "servers"],
    bearer: "brace",
    cmdArray: true,
    scopes: ["user", "project"],
  },
  vscode: { root: ["servers"], bearer: "literal", scopes: ["user", "project"] },
  antigravity: { root: ["mcpServers"], bearer: "literal", urlKey: "serverUrl", scopes: ["user", "project"] },
  grok: { root: ["mcp_servers"], bearer: "dollar", scopes: ["user", "project"] },
  cursor: { root: ["mcpServers"], bearer: "literal", scopes: ["user"] },
  cline: { root: ["mcpServers"], bearer: "literal", scopes: ["user"] },
  "gemini-cli": { root: ["mcpServers"], bearer: "literal", scopes: ["user"] },
  "claude-desktop": { root: ["mcpServers"], bearer: "literal", scopes: ["user"] },
  windsurf: { root: ["mcpServers"], bearer: "dollarEnv", urlKey: "serverUrl", scopes: ["user"] },
  trae: { root: ["mcpServers"], bearer: "literal", scopes: ["user", "project"] },
};

const CLIENTS = [
  ["claude-code", "Claude Code", "CLI", "JSON"],
  ["codex", "Codex CLI", "CLI", "TOML"],
  ["workbuddy", "WorkBuddy", "CLI", "JSON"],
  ["kimi-code", "Kimi Code", "CLI", "JSON"],
  ["opencode", "OpenCode", "CLI", "JSONC"],
  ["vscode", "VS Code", "IDE", "JSON"],
  ["antigravity", "Antigravity", "IDE", "JSON"],
  ["grok", "Grok", "CLI", "TOML"],
  ["cursor", "Cursor", "IDE", "JSON"],
  ["cline", "Cline", "IDE", "JSON"],
  ["gemini-cli", "Gemini CLI", "CLI", "JSON"],
  ["claude-desktop", "Claude Desktop", "桌面", "JSON"],
  ["windsurf", "Windsurf", "IDE", "JSON"],
  ["trae", "Trae", "IDE", "JSON"],
] as const;

interface MockEntry {
  name: string;
  transport: "stdio" | "http";
  scope: "inherit" | "user" | "project" | "local";
  command?: string;
  args?: string[];
  url?: string;
  tokenEnv?: string;
  env?: Record<string, string>;
  desc: string;
}

const ENTRIES: MockEntry[] = [
  {
    name: "stockdb",
    transport: "stdio",
    scope: "user",
    command: "c:/Dev/stockdata/.venv/Scripts/python.exe",
    args: ["-u", "C:/Dev/stockdata/ai_mcp/stockdb_full_mcp.py"],
    desc: "本地行情数据库",
  },
  {
    name: "github",
    transport: "http",
    scope: "project",
    url: "https://api.githubcopilot.com/mcp/",
    tokenEnv: "GITHUB_PAT",
    desc: "GitHub 仓库操作",
  },
  {
    name: "context7",
    transport: "http",
    scope: "inherit",
    url: "https://mcp.context7.com/mcp",
    tokenEnv: "CONTEXT7_API_KEY",
    desc: "文档检索",
  },
  {
    name: "playwright",
    transport: "stdio",
    scope: "local",
    command: "npx",
    args: ["-y", "@playwright/mcp@latest"],
    desc: "浏览器自动化",
  },
  {
    name: "filesystem",
    transport: "stdio",
    scope: "inherit",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-filesystem", "C:/Dev/mcpconfig"],
    desc: "文件系统访问",
  },
];

/** 已有的配置状态（user 作用域） */
const PRE: Record<string, "ok" | "diff"> = {
  "stockdb::claude-code": "ok",
  "stockdb::codex": "ok",
  "stockdb::workbuddy": "ok",
  "stockdb::kimi-code": "ok",
  "stockdb::opencode": "ok",
  "context7::claude-code": "ok",
  "github::codex": "diff",
  "playwright::claude-code": "ok",
};

function effScope(e: MockEntry, scope: string) {
  return e.scope === "inherit" ? scope : e.scope;
}

function bearerText(id: string, env: string) {
  const s = STYLES[id];
  switch (s.bearer) {
    case "native":
      return `bearer_token_env_var = "${env}"`;
    case "dollar":
      return `"Authorization": "Bearer \${${env}}"`;
    case "dollarEnv":
      return `"Authorization": "Bearer \${env:${env}}"`;
    case "brace":
      return `"Authorization": "Bearer {env:${env}}"`;
    default:
      return `"Authorization": "Bearer <需内联:${env}>"`;
  }
}

function snippet(id: string, e: MockEntry): string {
  const s = STYLES[id];
  const lines: string[] = [];
  if (e.transport === "http") {
    const urlKey = s.urlKey ?? "url";
    if (s.root[0] === "mcp_servers") {
      lines.push(`[mcp_servers.${e.name}]`);
      lines.push(`url = "${e.url}"`);
    } else {
      lines.push(`"${e.name}": {`);
      if (s.root[0] === "mcp") lines.push(`  "type": "remote",`);
      else lines.push(`  "type": "http",`);
      lines.push(`  "${urlKey}": "${e.url}",`);
    }
    if (e.tokenEnv) lines.push(`  ${bearerText(id, e.tokenEnv)}`);
    lines.push("}");
  } else {
    if (s.cmdArray) {
      lines.push(`"${e.name}": {`);
      lines.push(`  "type": "local",`);
      lines.push(`  "command": [${[e.command!, ...(e.args ?? [])].map((x) => `"${x}"`).join(", ")}]`);
      lines.push("}");
    } else if (s.root[0] === "mcp_servers") {
      lines.push(`[mcp_servers.${e.name}]`);
      lines.push(`command = "${e.command}"`);
      lines.push(`args = [${(e.args ?? []).map((x) => `"${x}"`).join(", ")}]`);
    } else {
      lines.push(`"${e.name}": {`);
      lines.push(`  "command": "${e.command}",`);
      lines.push(`  "args": [${(e.args ?? []).map((x) => `"${x}"`).join(", ")}]`);
      lines.push("}");
    }
  }
  return lines.map((l) => "+" + l).join("\n");
}

const BACKUPS = [
  {
    id: "20261026-070423",
    createdAt: "2026-10-26T07:04:23Z",
    files: 3,
    paths: ["~/.claude.json", "~/.codex/config.toml", "~/.config/opencode/opencode.json"],
  },
  {
    id: "20261025-201105",
    createdAt: "2026-10-25T20:11:05Z",
    files: 1,
    paths: ["~/.claude.json"],
  },
];

function selectedPlan(args: Record<string, unknown> | undefined, scope: Scope): PlanDto {
  const selection = args?.selection as McpSelection | undefined;
  if (!selection) throw new Error("必须提供 MCP 选择");
  const scopedEntries = ENTRIES.filter((e) => effScope(e, scope) === scope);
  const files: PlanDto["files"] = [];
  const skipped: PlanDto["skipped"] = [];
  for (const [id, names] of Object.entries(selection)) {
    const client = CLIENTS.find(([cid]) => cid === id);
    if (!client) throw new Error(`未知客户端：${id}`);
    if (names.some((name) => !scopedEntries.some((e) => e.name === name))) {
      throw new Error("选择包含不存在或不属于当前作用域的 MCP");
    }
    if (names.length === 0) continue;
    if (!STYLES[id].scopes.includes(scope)) {
      skipped.push({ clientId: id, reason: `${client[1]} 不支持此作用域` });
      continue;
    }
    const entries = scopedEntries.filter((e) => names.includes(e.name));
    files.push({
      clientId: id,
      clientName: client[1],
      path: `~/${id}/mcp.json`,
      format: "JSON",
      scope,
      existed: true,
      blocked: null,
      newCount: entries.length,
      updateCount: 0,
      unchangedCount: 0,
      diff: entries.map((e) => snippet(id, e)).join("\n"),
      warnings: [],
      willWrite: true,
    });
  }
  return {
    entryCount: new Set(Object.values(selection).flat()).size,
    issues: [],
    missingVars: [],
    skipped,
    files,
    writable: files.length,
  };
}

export async function mockCall<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  await new Promise((r) => setTimeout(r, 120));
  const scope = (args?.scope as Scope) ?? "user";

  switch (cmd) {
    case "load_settings":
    case "default_settings": return structuredClone(settings) as T;
    case "save_settings": settings = structuredClone(args?.settings as GuiSettings); return [] as T;
    case "choose_directory": return "C:/Dev/demo-project" as T;
    case "create_catalog": return [] as T;
    case "scan_import": {
      const scan: ImportScan = { scanId: `demo-scan-${++importSerial}`, sourcePath: `~/${args?.clientId}/mcp.json`, catalogPath: String(args?.catalogPath), catalogExists: true,
        items: [{ name: "example-mcp", transport: "stdio", conflict: ENTRIES.some((entry) => entry.name === "example-mcp"), blocked: null,
          fields: [{ id: "/args/0", label: "参数 1" }, { id: "/env/API_KEY", label: "env.API_KEY" }] }] };
      importScans.set(scan.scanId, scan); return scan as T;
    }
    case "cancel_import": {
      importScans.delete(String(args?.scanId));
      for (const [id, preview] of importPreviews) if (preview.scanId === args?.scanId) importPreviews.delete(id);
      return undefined as T;
    }
    case "preview_import": {
      const scan = importScans.get(String(args?.scanId)); if (!scan) throw new Error("导入草稿已失效");
      for (const [id, preview] of importPreviews) if (preview.scanId === scan.scanId) importPreviews.delete(id);
      const choices = (args?.choices as ImportChoice[]).filter((choice) => choice.action !== "skip");
      if (!choices.length) throw new Error("尚未选择可导入的条目");
      const changes = choices.map((choice) => {
        const item = scan.items.find((item) => item.name === choice.sourceName);
        if (!item || !choice.targetName.trim() || item.fields.some((field) => !/^[A-Za-z_][A-Za-z0-9_]*$/.test(choice.bindings[field.id] ?? ""))) throw new Error("请完整填写环境变量映射");
        const exists = ENTRIES.some((entry) => entry.name === choice.targetName);
        if (choice.action === "replace" ? !exists : exists) throw new Error("目标名称冲突，请重新处理");
        return { sourceName: choice.sourceName, targetName: choice.targetName, action: choice.action,
          before: choice.action === "replace" ? { command: "[已隐藏]" } : null,
          after: { name: choice.targetName, transport: "stdio", command: "python", args: [`\${ENV:${choice.bindings['/args/0']}}`], env: { API_KEY: `\${ENV:${choice.bindings['/env/API_KEY']}}` }, scope } };
      });
      const preview = { previewId: `demo-preview-${++importSerial}`, scanId: scan.scanId, scope, catalogPath: scan.catalogPath, changes };
      importPreviews.set(preview.previewId, preview); return preview as T;
    }
    case "commit_import": {
      const preview = importPreviews.get(String(args?.previewId)); if (!preview) throw new Error("预览已失效");
      importPreviews.delete(preview.previewId); importScans.delete(preview.scanId);
      for (const change of preview.changes) {
        const index = ENTRIES.findIndex((entry) => entry.name === change.targetName);
        if (index >= 0) ENTRIES.splice(index, 1);
        const entry = change.after as { name: string; command: string; args: string[]; env: Record<string, string> };
        ENTRIES.push({ ...entry, transport: "stdio", scope: preview.scope, desc: "从 Agent 导入" });
      }
      return { imported: preview.changes.length, warnings: [] } as T;
    }
    case "app_info":
      return {
        version: "0.3.0",
        platform: "windows",
        home: "C:/Users/Chensh",
        projectRoot: "C:/Dev/mcpconfig",
        dataDir: "C:/Users/Chensh/.mcp-hub",
      } as T;

    case "detect":
      return CLIENTS.map(([id, name, kind, format]) => {
        const s = STYLES[id];
        const block = !s.scopes.includes(scope)
          ? `${name} 不支持${scope === "user" ? "用户级" : scope === "project" ? "项目级" : "本地级"}`
          : null;
        const known = ["claude-code", "codex", "kimi-code", "opencode", "cline", "antigravity", "grok"];
        return {
          id,
          name,
          kind,
          format,
          state: known.includes(id) ? "ready" : id === "vscode" ? "installed" : "missing",
          stateLabel: known.includes(id) ? "已就绪" : id === "vscode" ? "已安装未配置" : "未检测",
          binary: known.includes(id) ? `/usr/local/bin/${id}` : null,
          path: `~/${id}/mcp.json`,
          configured: known.includes(id) ? 3 : 0,
          scopes: s.scopes,
          scopeBlock: block,
        };
      }) as T;

    case "read_client_config": {
      const client = CLIENTS.find(([id]) => id === args?.clientId);
      if (!client) throw new Error("未知客户端");
      if (!STYLES[client[0]].scopes.includes(scope) || (client[0] === "claude-code" && scope === "local")) {
        throw new Error("不支持当前作用域");
      }
      return {
        clientId: client[0], clientName: client[1], scope,
        path: `~/${client[0]}/mcp.json`, format: client[3], existed: true,
        entries: [{ name: "example-mcp", config: {
          command: "python", args: ["[已隐藏]"], env: { API_KEY: "[已隐藏]" },
        } }],
      } as T;
    }

    case "matrix": {
      const rows = ENTRIES.filter((e) => effScope(e, scope) === scope).map((e) => ({
        name: e.name,
        transport: e.transport,
        scope: effScope(e, scope),
        cells: CLIENTS.map(([id, name]) => {
          const s = STYLES[id];
          if (!s.scopes.includes(scope)) {
            return {
              clientId: id,
              state: "unsupported",
              detail: `${name} 不支持此作用域`,
            };
          }
          const pre = PRE[`${e.name}::${id}`];
          return {
            clientId: id,
            state: pre ?? "absent",
            detail: null,
          };
        }),
      }));
      return {
        platform: "windows",
        scope,
        clients: CLIENTS.map(([id, name, kind]) => ({ id, name, kind })),
        rows,
        issues: [],
      } as T;
    }

    case "load_catalog":
      return {
        exists: true,
        catalog: { version: 1, entries: ENTRIES.map(({ desc, tokenEnv, ...entry }) => ({ ...entry, description: desc, ...(tokenEnv ? { auth: { type: "bearer", token_env: tokenEnv } } : {}) })) },
        issues: [],
        entryCount: ENTRIES.length,
      } as T;

    case "save_catalog": {
      const catalog = args?.catalog as { entries: any[] };
      ENTRIES.splice(0, ENTRIES.length, ...catalog.entries.map((entry) => ({ ...entry, desc: entry.description ?? "", tokenEnv: entry.auth?.token_env, scope: entry.scope ?? "inherit" })));
      return [] as unknown as T;
    }

    case "plan": {
      return selectedPlan(args, scope) as T;
    }

    case "apply": {
      const plan = selectedPlan(args, scope);
      const dryRun = args?.dryRun === true;
      return {
        written: dryRun ? 0 : plan.writable,
        blocked: 0,
        failed: 0,
        backupId: dryRun || !plan.writable ? null : "20261026-071500",
        outcomes: plan.files.map((file) => ({
          clientId: file.clientId,
          path: file.path,
          status: dryRun ? "dryRun" : "written",
          label: dryRun ? "演练" : "已写入",
          message: null,
        })),
      } as T;
    }

    case "backups":
      return BACKUPS as T;

    case "restore":
      return 3 as T;
  }
  throw new Error(`mock: 未实现的命令 ${cmd}`);
}
