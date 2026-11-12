import type { Entry } from "./pages/Catalog";

export interface SnippetParseResult {
  entry: Partial<Entry>;
  /** 当粘贴内容包含多个 MCP 条目时提示共识别到几项（默认取第 1 项填入） */
  totalFound: number;
  sourceKind: "json" | "cli" | "url";
}

/** 剥离 JSONC 的行注释、块注释与尾逗号（跳过双引号字符串内部）。 */
function stripJsonc(src: string): string {
  let out = "";
  let inStr = false;
  let esc = false;
  for (let i = 0; i < src.length; i++) {
    const ch = src[i];
    if (inStr) {
      out += ch;
      if (esc) esc = false;
      else if (ch === "\\") esc = true;
      else if (ch === '"') inStr = false;
      continue;
    }
    if (ch === '"') {
      inStr = true;
      out += ch;
      continue;
    }
    if (ch === "/" && src[i + 1] === "/") {
      while (i < src.length && src[i] !== "\n") i++;
      if (i < src.length) out += "\n";
      continue;
    }
    if (ch === "/" && src[i + 1] === "*") {
      i += 2;
      while (i < src.length && !(src[i] === "*" && src[i + 1] === "/")) i++;
      i++;
      continue;
    }
    out += ch;
  }
  let clean = "";
  inStr = false;
  esc = false;
  for (let i = 0; i < out.length; i++) {
    const ch = out[i];
    if (inStr) {
      clean += ch;
      if (esc) esc = false;
      else if (ch === "\\") esc = true;
      else if (ch === '"') inStr = false;
      continue;
    }
    if (ch === '"') {
      inStr = true;
      clean += ch;
      continue;
    }
    if (ch === ",") {
      let j = i + 1;
      while (j < out.length && /\s/.test(out[j])) j++;
      if (out[j] === "}" || out[j] === "]") continue;
    }
    clean += ch;
  }
  return clean;
}

/** 归一化常见客户端环境变量占位符为 MCP Hub 统一语法 `${ENV:VAR}`。 */
function normalizeEnvRefs(val: string): string {
  return val
    .replace(/\{env:([A-Za-z_][A-Za-z0-9_]*)\}/g, "${ENV:$1}")
    .replace(/\$\{env:([A-Za-z_][A-Za-z0-9_]*)\}/g, "${ENV:$1}")
    .replace(/\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (_m, name: string) =>
      ["HOME", "HOME_DIR", "PROJECT_ROOT", "CWD"].includes(name) ? `\${${name}}` : `\${ENV:${name}}`,
    );
}

function sanitizeName(raw: string): string {
  return raw
    .trim()
    .replace(/^@[^/]+\//, "")
    .replace(/@[0-9A-Za-z._-]+$/, "")
    .replace(/^(mcp-server-|server-|mcp-)/i, "")
    .replace(/(-mcp|-server)$/i, "")
    .replace(/[^A-Za-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .toLowerCase();
}

function inferNameFromCommand(command: string, args: string[]): string {
  for (const arg of args) {
    if (arg.startsWith("-")) continue;
    if (arg.startsWith("@") || /mcp|server/i.test(arg) || /^[a-z0-9][\w.-]*\/[\w.-]+$/i.test(arg)) {
      const candidate = sanitizeName(arg.split("/").pop() || arg);
      if (candidate) return candidate;
    }
  }
  for (const arg of args) {
    if (!arg.startsWith("-") && /\.(py|js|mjs|cjs|ts)$/i.test(arg)) {
      const base = arg.replace(/\\/g, "/").split("/").pop()?.replace(/\.[a-z]+$/i, "") ?? "";
      const candidate = sanitizeName(base);
      if (candidate) return candidate;
    }
  }
  const baseCmd = command.replace(/\\/g, "/").split("/").pop()?.replace(/\.(exe|cmd|bat)$/i, "") ?? "";
  if (baseCmd && !["npx", "uvx", "node", "python", "python3", "pnpx", "bunx", "cmd", "sh"].includes(baseCmd.toLowerCase())) {
    return sanitizeName(baseCmd);
  }
  return "";
}

function inferNameFromUrl(url: string): string {
  try {
    const u = new URL(url);
    const parts = u.hostname.split(".").filter((p) => !["www", "api", "mcp", "com", "org", "io", "dev", "net", "ai"].includes(p.toLowerCase()));
    if (parts.length > 0) return sanitizeName(parts[0]);
    const seg = u.pathname.split("/").filter((p) => p && p.toLowerCase() !== "mcp" && p.toLowerCase() !== "sse")[0];
    if (seg) return sanitizeName(seg);
  } catch {
    // ignore URL parse error
  }
  return "";
}

function isServerLikeObject(obj: unknown): boolean {
  if (!obj || typeof obj !== "object" || Array.isArray(obj)) return false;
  const o = obj as Record<string, unknown>;
  return (
    typeof o.command === "string" ||
    Array.isArray(o.command) ||
    typeof o.url === "string" ||
    typeof o.serverUrl === "string" ||
    typeof o.httpUrl === "string"
  );
}

function mapObjectToEntry(rawName: string | undefined, obj: Record<string, unknown>): Partial<Entry> {
  const out: Partial<Entry> = {};

  // 传输识别
  const rawUrl =
    typeof obj.url === "string"
      ? obj.url
      : typeof obj.serverUrl === "string"
        ? obj.serverUrl
        : typeof obj.httpUrl === "string"
          ? obj.httpUrl
          : undefined;

  if (rawUrl !== undefined) {
    const rawType = typeof obj.type === "string" ? obj.type.toLowerCase() : typeof obj.transport === "string" ? obj.transport.toLowerCase() : "";
    out.transport = rawType === "sse" || /\/sse\/?$/i.test(rawUrl) ? "sse" : "http";
    out.url = normalizeEnvRefs(rawUrl.trim());

    const rawHeaders = (obj.headers ?? obj.httpHeaders) as Record<string, unknown> | undefined;
    const headers: Record<string, string> = {};
    if (rawHeaders && typeof rawHeaders === "object" && !Array.isArray(rawHeaders)) {
      for (const [k, v] of Object.entries(rawHeaders)) {
        if (v !== undefined && v !== null) headers[k] = normalizeEnvRefs(String(v));
      }
    }

    // 识别 Bearer 或自定义认证字段
    const bearerEnv =
      typeof obj.bearer_token_env_var === "string"
        ? obj.bearer_token_env_var
        : typeof (obj.auth as Record<string, unknown> | undefined)?.token_env === "string"
          ? String((obj.auth as Record<string, unknown>).token_env)
          : undefined;

    if (bearerEnv) {
      out.auth = { type: "bearer", token_env: bearerEnv.trim() };
    } else {
      const authKey = Object.keys(headers).find((k) => k.toLowerCase() === "authorization");
      if (authKey) {
        const m = headers[authKey].match(/^Bearer\s+\$\{ENV:([A-Za-z_][A-Za-z0-9_]*)\}$/i);
        if (m) {
          out.auth = { type: "bearer", token_env: m[1] };
          delete headers[authKey];
        }
      }
    }
    if (Object.keys(headers).length > 0) out.headers = headers;
  } else {
    out.transport = "stdio";
    if (Array.isArray(obj.command)) {
      const parts = obj.command.map((x) => normalizeEnvRefs(String(x)));
      out.command = parts[0] ?? "";
      out.args = parts.slice(1);
    } else if (typeof obj.command === "string") {
      out.command = normalizeEnvRefs(obj.command.trim());
      out.args = Array.isArray(obj.args) ? obj.args.map((x) => normalizeEnvRefs(String(x))) : [];
    }

    const rawEnv = (obj.env ?? obj.environment) as Record<string, unknown> | undefined;
    if (rawEnv && typeof rawEnv === "object" && !Array.isArray(rawEnv)) {
      const env: Record<string, string> = {};
      for (const [k, v] of Object.entries(rawEnv)) {
        if (v !== undefined && v !== null) env[k] = normalizeEnvRefs(String(v));
      }
      if (Object.keys(env).length > 0) out.env = env;
    }

    if (typeof obj.cwd === "string" && obj.cwd.trim()) {
      out.cwd = normalizeEnvRefs(obj.cwd.trim());
    }
  }

  if (typeof obj.description === "string" && obj.description.trim()) {
    out.description = obj.description.trim();
  }
  if (typeof obj.disabled === "boolean") {
    out.enabled = !obj.disabled;
  } else if (typeof obj.enabled === "boolean") {
    out.enabled = obj.enabled;
  }
  const timeoutSec =
    typeof obj.timeout_seconds === "number"
      ? obj.timeout_seconds
      : typeof obj.startup_timeout_sec === "number"
        ? obj.startup_timeout_sec
        : typeof obj.timeout === "number"
          ? obj.timeout > 1000
            ? Math.round(obj.timeout / 1000)
            : obj.timeout
          : undefined;
  if (timeoutSec && timeoutSec > 0) {
    out.timeout_seconds = Math.floor(timeoutSec);
  }

  const chosenName =
    (rawName && rawName.trim()) ||
    (typeof obj.name === "string" ? obj.name.trim() : "") ||
    (out.transport === "stdio" ? inferNameFromCommand(out.command ?? "", out.args ?? []) : inferNameFromUrl(out.url ?? ""));
  if (chosenName) out.name = chosenName;

  return out;
}

/** 分词解析 Shell 命令行（支持单双引号与反斜杠转义）。 */
function splitShellArgs(line: string): string[] {
  const cleaned = line.replace(/\\\r?\n/g, " ").trim();
  const tokens: string[] = [];
  let cur = "";
  let quote: '"' | "'" | null = null;
  for (let i = 0; i < cleaned.length; i++) {
    const ch = cleaned[i];
    if (quote) {
      if (ch === quote) {
        quote = null;
      } else if (ch === "\\" && quote === '"' && i + 1 < cleaned.length && ['"', "\\", "$", "`"].includes(cleaned[i + 1])) {
        cur += cleaned[++i];
      } else {
        cur += ch;
      }
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      continue;
    }
    if (/\s/.test(ch)) {
      if (cur.length > 0) {
        tokens.push(cur);
        cur = "";
      }
      continue;
    }
    if (ch === "\\" && i + 1 < cleaned.length && /\s|["']/.test(cleaned[i + 1])) {
      cur += cleaned[++i];
      continue;
    }
    cur += ch;
  }
  if (cur.length > 0) tokens.push(cur);
  return tokens;
}

/**
 * 解析常见 MCP 配置片段（JSON / JSONC / 键值片段 / `claude mcp add` / `npx` 命令行 / 远程 URL）。
 */
export function parseMcpSnippet(raw: string): SnippetParseResult {
  const trimmed = raw
    .trim()
    .replace(/^```[a-zA-Z]*\r?\n/, "")
    .replace(/\r?\n```$/, "")
    .trim();

  if (!trimmed) {
    throw new Error("请先粘贴 JSON 配置片段或启动命令");
  }

  // 1. 纯 URL
  if (/^https?:\/\/\S+$/i.test(trimmed)) {
    const transport: Entry["transport"] = /\/sse\/?$/i.test(trimmed) ? "sse" : "http";
    return {
      entry: {
        name: inferNameFromUrl(trimmed),
        transport,
        url: normalizeEnvRefs(trimmed),
      },
      totalFound: 1,
      sourceKind: "url",
    };
  }

  // 2. JSON / JSONC 或 `"serverName": { ... }` 片段
  const looksLikeNamedFragment = /^"[^"]+"\s*:\s*\{[\s\S]*\}\s*,?$/.test(trimmed);
  if (trimmed.startsWith("{") || looksLikeNamedFragment) {
    const candidateJson = looksLikeNamedFragment
      ? `{${trimmed.replace(/,\s*$/, "")}}`
      : trimmed.replace(/,\s*$/, "");
    let parsed: unknown;
    try {
      parsed = JSON.parse(stripJsonc(candidateJson));
    } catch {
      throw new Error("JSON 语法解析失败，请检查括号或引号是否完整");
    }
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      throw new Error("JSON 根节点必须是配置对象");
    }
    const root = parsed as Record<string, unknown>;

    // 情况 A：本身就是单个 server 定义
    if (isServerLikeObject(root)) {
      return {
        entry: mapObjectToEntry(undefined, root),
        totalFound: 1,
        sourceKind: "json",
      };
    }

    // 情况 B：包含 mcpServers / servers / mcp_servers / mcp.servers / mcp
    const containerCandidates: unknown[] = [
      root.mcpServers,
      root.servers,
      root.mcp_servers,
      root.mcp && typeof root.mcp === "object" && !Array.isArray(root.mcp)
        ? (root.mcp as Record<string, unknown>).servers ?? root.mcp
        : undefined,
      root,
    ];
    for (const container of containerCandidates) {
      if (!container || typeof container !== "object" || Array.isArray(container)) continue;
      const pairs = Object.entries(container as Record<string, unknown>).filter(([, v]) => isServerLikeObject(v));
      if (pairs.length > 0) {
        const [firstName, firstObj] = pairs[0];
        return {
          entry: mapObjectToEntry(firstName, firstObj as Record<string, unknown>),
          totalFound: pairs.length,
          sourceKind: "json",
        };
      }
    }
    throw new Error("未在 JSON 中找到有效的 MCP 服务字段（需包含 command 或 url）");
  }

  // 3. CLI 命令（支持 `claude mcp add ...` 或 `[KEY=VAL ...] npx -y @scope/pkg ...`）
  const tokens = splitShellArgs(trimmed.replace(/^\$\s+/, ""));
  if (tokens.length === 0) {
    throw new Error("未能从输入中解析出有效命令");
  }

  // 剥离前导 `env` 或 `KEY=VALUE` 环境变量前缀
  const env: Record<string, string> = {};
  let idx = 0;
  if (tokens[idx] === "env") idx++;
  while (idx < tokens.length && /^[A-Za-z_][A-Za-z0-9_]*=/.test(tokens[idx])) {
    const eq = tokens[idx].indexOf("=");
    const k = tokens[idx].slice(0, eq);
    const v = normalizeEnvRefs(tokens[idx].slice(eq + 1));
    env[k] = v;
    idx++;
  }

  // 识别 `claude mcp add`
  if (tokens[idx] === "claude" && tokens[idx + 1] === "mcp" && tokens[idx + 2] === "add") {
    idx += 3;
    let transport: Entry["transport"] = "stdio";
    let scope: Entry["scope"] | undefined;
    const headers: Record<string, string> = {};
    const positionals: string[] = [];
    let afterDashDash = false;
    const dashArgs: string[] = [];

    while (idx < tokens.length) {
      const tok = tokens[idx];
      if (afterDashDash) {
        dashArgs.push(tok);
        idx++;
        continue;
      }
      if (tok === "--") {
        afterDashDash = true;
        idx++;
        continue;
      }
      if ((tok === "-t" || tok === "--transport") && tokens[idx + 1]) {
        const t = tokens[++idx].toLowerCase();
        if (t === "http" || t === "sse" || t === "stdio") transport = t;
        idx++;
        continue;
      }
      if ((tok === "-s" || tok === "--scope") && tokens[idx + 1]) {
        const s = tokens[++idx].toLowerCase();
        if (s === "user" || s === "project" || s === "local") scope = s;
        idx++;
        continue;
      }
      if ((tok === "-e" || tok === "--env") && tokens[idx + 1]) {
        const kv = tokens[++idx];
        const eq = kv.indexOf("=");
        if (eq > 0) env[kv.slice(0, eq)] = normalizeEnvRefs(kv.slice(eq + 1));
        idx++;
        continue;
      }
      if ((tok === "-H" || tok === "--header") && tokens[idx + 1]) {
        const hv = tokens[++idx];
        const colon = hv.indexOf(":");
        if (colon > 0) headers[hv.slice(0, colon).trim()] = normalizeEnvRefs(hv.slice(colon + 1).trim());
        idx++;
        continue;
      }
      positionals.push(tok);
      idx++;
    }

    const name = positionals[0] ?? "";
    const rest = dashArgs.length > 0 ? dashArgs : positionals.slice(1);
    if (transport === "http" || transport === "sse" || (rest[0] && /^https?:\/\//i.test(rest[0]))) {
      const url = normalizeEnvRefs(rest[0] ?? "");
      return {
        entry: {
          name: name || inferNameFromUrl(url),
          transport: transport === "stdio" ? (/\/sse\/?$/i.test(url) ? "sse" : "http") : transport,
          url,
          ...(Object.keys(headers).length ? { headers } : {}),
          ...(scope ? { scope } : {}),
        },
        totalFound: 1,
        sourceKind: "cli",
      };
    }

    const command = rest[0] ?? "";
    const args = rest.slice(1).map(normalizeEnvRefs);
    if (!command) throw new Error("claude mcp add 命令缺少启动程序或远程 URL");
    return {
      entry: {
        name: name || inferNameFromCommand(command, args),
        transport: "stdio",
        command: normalizeEnvRefs(command),
        args,
        ...(Object.keys(env).length ? { env } : {}),
        ...(scope ? { scope } : {}),
      },
      totalFound: 1,
      sourceKind: "cli",
    };
  }

  // 普通命令行：`npx -y @playwright/mcp@latest`
  const command = tokens[idx];
  if (!command) throw new Error("未能识别启动命令");
  const args = tokens.slice(idx + 1).map(normalizeEnvRefs);
  return {
    entry: {
      name: inferNameFromCommand(command, args),
      transport: "stdio",
      command: normalizeEnvRefs(command),
      args,
      ...(Object.keys(env).length ? { env } : {}),
    },
    totalFound: 1,
    sourceKind: "cli",
  };
}

/** 生成单行紧凑技术规格摘要（供主列表与详情抽屉直接展示命令/URL，已脱敏安全）。 */
export function summarizeEntryEndpoint(entry: Entry): string {
  if (entry.transport === "stdio") {
    const cmd = (entry.command ?? "").trim();
    if (!cmd) return "未设置命令";
    const shortCmd = cmd.includes("/") || cmd.includes("\\") ? cmd.replace(/\\/g, "/").split("/").pop() || cmd : cmd;
    const args = (entry.args ?? []).join(" ");
    return args ? `${shortCmd} ${args}` : shortCmd;
  }
  return (entry.url ?? "").trim() || "未设置 URL";
}

/** 生成完整可复制命令或远程地址规格文本。 */
export function formatEntryFullCommand(entry: Entry): string {
  if (entry.transport === "stdio") {
    const parts = [entry.command ?? "", ...(entry.args ?? [])].filter(Boolean);
    return parts
      .map((p) => (/\s/.test(p) ? `"${p.replace(/"/g, '\\"')}"` : p))
      .join(" ");
  }
  return entry.url ?? "";
}
