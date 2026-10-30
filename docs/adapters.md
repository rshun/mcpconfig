# 各 Agent 适配器细则

> 配套 `ARCHITECTURE.md`。本文档记录每个 Agent 的**实测/官方核实**细节，作为适配器实现的依据。
> 标注 `[实测]` 的来自本机文件读取；标注 `[官方]` 的来自官方文档。

---

## 通用约定

### 根键与作用域

| 作用域 | 含义 | 优先级 |
|---|---|---|
| User | 用户级，所有项目生效 | 最低 |
| Project | 项目根目录，随仓库分发 | 中 |
| Local | 仅本机本项目，不随仓库 | 最高 |

Claude Code 三层作用域相互独立：同名条目在多层同时存在时，**取最高优先级来源的整条配置，不做字段级合并**。
其余 Agent（Codex / Grok / OpenCode）多为「项目层完全覆盖用户层同名条目」。

### 传输类型映射

| IR Transport | 多数 Agent | Codex (TOML) | Antigravity | OpenCode |
|---|---|---|---|---|
| Stdio | `command` + `args[]` | `command` + `args[]` | `command` + `args[]` | `command[]`（数组） |
| Http | `url` + `type:"http"` | `url` | `serverUrl` | `url` + `type:"remote"` |
| Sse | `url` + `type:"sse"` | `url` | 不支持（仅 streamable-http） | `url` + `type:"remote"` |

Antigravity 官方明确：**远程服务器必须使用 Streamable HTTP，`serverUrl` 与 `url` 都支持，但不支持 `httpUrl` 旧字段；不支持 HTTP+SSE 传统传输**。

---

## 1. WorkBuddy

| 项 | 值 | 来源 |
|---|---|---|
| 用户级路径 | `~/.workbuddy/mcp.json` | [实测] |
| 格式 | JSON | [实测] |
| 根键 | `mcpServers` | [实测] |
| stdio | `{command, args[], cwd, env}` | [实测] |
| remote | `{type:"http", url, headers}` | [实测] |

`cwd` 字段是 WorkBuddy 特有，其他 Agent 未必支持 —— `supports_cwd` 按适配器声明。

---

## 2. Claude Code

| 项 | 值 | 来源 |
|---|---|---|
| 用户级路径 | `~/.claude.json`（顶层 `mcpServers`） | [官方] |
| 项目级路径 | `<proj>/.mcp.json`（`mcpServers`） | [官方] |
| 本机作用域 | `~/.claude.json` 内按项目路径的嵌套条目 | [官方] |
| 格式 | JSON | [官方] |
| stdio | `{type:"stdio", command, args[], env}` | [官方] |
| remote | `{type:"http"\|"sse", url, headers}` | [官方] |
| 环境变量 | `CLAUDE_CONFIG_DIR` 可改写 `~/.claude.json` 位置 | [官方] |

要点：
- `type` 字段接受 `streamable-http` 作为 `http` 的别名
- 项目级条目首次使用需批准；`enableAllProjectMcpServers` 在不可信目录中会被忽略
- `claude mcp add-json` 与文件格式一致

---

## 3. Codex CLI

| 项 | 值 | 来源 |
|---|---|---|
| 用户级路径 | `~/.codex/config.toml` | [实测][官方] |
| 项目级路径 | `<proj>/.codex/config.toml`（仅受信任项目） | [官方] |
| 格式 | **TOML** | [实测] |
| 根键 | `[mcp_servers.<name>]` | [实测] |
| stdio | `command`, `args[]`, `env{}`, `env_vars[]`, `cwd` | [官方] |
| remote | `url`, `bearer_token_env_var`, `http_headers{}`, `env_http_headers{}`, `auth="oauth"` | [官方] |
| 其他 | `startup_timeout_sec`, `tool_timeout_sec`, `enabled`, `enabled_tools[]`, `disabled_tools[]` | [官方] |

**必须用 `toml_edit` 而非 `toml` crate** —— 实测本机 `~/.codex/config.toml` 含大量注释、`[marketplaces.*]`、`[plugins.*]`、`[shell_environment_policy.set]` 等无关段落，用 `toml` 序列化会全部丢失。

`env` 与 `env_vars` 语义不同：`env` 写死在文件里，`env_vars` 转发已存在的环境变量 —— **密钥应优先映射到 `env_vars` / `bearer_token_env_var`**。

---

## 4. Kimi Code

| 项 | 值 | 来源 |
|---|---|---|
| 用户级路径 | `~/.kimi-code/mcp.json` | [实测] |
| 格式 | JSON | [实测] |
| 根键 | `mcpServers` | [实测] |
| stdio | `{command, args[], cwd}` | [实测] |

实测样本：
```json
{"mcpServers":{"stockdb-native":{"command":"c:/Dev/stockdata/.venv/Scripts/python.exe","args":["-u","c:/Dev/stockdata/ai_mcp/stockdb_full_mcp.py"],"cwd":"c:/Dev/stockdata/ai_mcp"}}}
```

---

## 5. OpenCode

| 项 | 值 | 来源 |
|---|---|---|
| 全局路径 | `~/.config/opencode/opencode.json(c)` | [官方] |
| 项目路径 | `<proj>/opencode.json(c)` / `.opencode/config.json` | [官方] |
| 格式 | JSON / JSONC（支持注释与尾逗号） | [官方] |
| 根键 | **`mcp.servers`**（仅 v2） | [官方] |
| stdio | `{type:"local", command:[可执行, 参数...], environment{}, cwd}` | [官方] |
| remote | `{type:"remote", url, headers{}, oauth, timeout}` | [官方] |
| 开关 | `disabled: true`（**语义取反**，启用则不写该字段） | [官方] |

要点：
- **仅支持 v2（D2 决策）**：不做 v1 兼容、不做版本探测，根键固定 `mcp.servers`
- **仍须检测 v1 结构**：若命中文件是 v1 扁平形态（`mcp.<name>` 直接挂服务器），**提示用户迁移后重试**，不静默按 v2 写入 —— 否则 v1 客户端会读不到或读错
- **`command` 是数组且含可执行文件本身**，与其他客户端的 `command`+`args` 语义不同 → `EntryStyle::CommandArray`
- 环境变量键是 `environment` 而非 `env`
- 配置文件查找顺序：`~/.config/opencode/config.json` → `opencode.json` → `opencode.jsonc`，共 6 层优先级
- 支持 `{env:VAR}` 变量替换（不是 `$VAR`）

---

## 6. VS Code / GitHub Copilot

| 项 | 值 | 来源 |
|---|---|---|
| 用户级路径 | `%APPDATA%\Code\User\mcp.json`（根键 **`servers`**） | [官方] |
| 工作区路径 | `<proj>/.vscode/mcp.json`（根键 **`servers`**） | [官方] |
| 便携用户级 | `~/.copilot/mcp-config.json`（根键 `mcpServers`） | [官方] |
| 便携工作区 | `<proj>/.mcp.json`（根键 `mcpServers`） | [官方] |
| stdio | `{command, args[], env}` | [官方] |
| remote | `{type:"http", url}` | [官方] |

要点：
- **同一 IDE 两套根键**，取决于用哪种格式 → 适配器按目标路径选择根键
- Agent Host 场景不读 `.vscode/mcp.json`，但会读 `.mcp.json` 与 `~/.copilot/mcp-config.json`
- 官方建议新配置优先用便携格式（`.mcp.json` / `~/.copilot/mcp-config.json`）

---

## 7. Antigravity

| 项 | 值 | 来源 |
|---|---|---|
| 全局路径 | `~/.gemini/config/mcp_config.json` | [官方] |
| 工作区路径 | `<proj>/.agents/mcp_config.json` | [官方] |
| 格式 | JSON | [官方] |
| 根键 | `mcpServers` | [官方] |
| stdio | `command` + `args[]` | [官方] |
| remote | **`serverUrl`**（`url` 亦可）, `headers{}`, `timeoutSeconds`, `env{}` | [官方] |
| OAuth | `{oauth: {clientId, clientSecret}}` | [官方] |

要点：
- 远程必须 Streamable HTTP，`url`/`httpUrl` 旧字段不支持
- 由 Gemini CLI 迁移而来：原 `~/.gemini/settings.json` 内联的 `mcpServers` 需迁出到独立文件
- 另有 `~/.gemini/antigravity/mcp_config.json` 用于特定 OAuth 场景

---

## 8. Grok Build

| 项 | 值 | 来源 |
|---|---|---|
| 用户级路径 | `~/.grok/config.toml`（另有文档提及 `~/.grok/mcp.json`） | [官方] |
| 项目级路径 | `<proj>/.grok/config.toml` | [官方] |
| 格式 | **TOML** | [官方] |
| 根键 | `[mcp_servers.<name>]` | [官方] |
| stdio | `command`, `args[]`, `env{}` | [官方] |
| remote | `url`, `headers{}` | [官方] |
| 其他 | `startup_timeout_sec`（默认 30）, `tool_timeout_sec` | [官方] |

要点：
- 还会读取 `~/.claude.json`、`.cursor/mcp.json`、项目根 `.mcp.json` —— **写 Grok 配置时需注意与这些来源的重复**
- `${VAR}` 在 url / command / args / env / headers 中会展开
- 项目级条目**整体替换**用户级同名条目
- 路径存在版本差异（`config.toml` vs `mcp.json`），适配器需探测候选路径

---

## 9. Cursor

| 项 | 值 | 来源 |
|---|---|---|
| 用户级路径 | `~/.cursor/mcp.json` | [官方] |
| 格式 | JSON | [官方] |
| 根键 | `mcpServers` | [官方] |

---

## 10. Cline

| 项 | 值 | 来源 |
|---|---|---|
| 用户级路径 | `~/.cline/data/settings/cline_mcp_settings.json` | [实测] |
| 格式 | JSON | [实测] |
| 根键 | `mcpServers` | [实测] |

---

## 11. Gemini CLI

| 项 | 值 | 来源 |
|---|---|---|
| 用户级路径 | `~/.gemini/settings.json`（`mcpServers` 内联） | [官方] |
| 格式 | JSON | [官方] |

注意：Antigravity 已把 MCP 从该文件迁出到 `~/.gemini/config/mcp_config.json`，两者需分别处理。

---

## 12. Claude Desktop

| 项 | 值 | 来源 |
|---|---|---|
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` | [官方] |
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` | [官方] |
| Linux | `~/.config/Claude/claude_desktop_config.json` | [官方] |
| 格式 | JSON | [官方] |
| 根键 | `mcpServers` | [官方] |

---

## 13. Windsurf

| 项 | 值 | 来源 |
|---|---|---|
| 用户级路径 | `~/.codeium/windsurf/mcp_config.json`（另有文档提及 `~/.codeium/mcp_config.json`） | [官方] |
| 格式 | JSON | [官方] |
| 根键 | `mcpServers` | [官方] |
| stdio | `{command, args[], env{}}` | [官方] |
| remote | `{url}` 或 **`{serverUrl}`**（两者都可用）, `headers{}` | [官方] |
| 变量插值 | **`${env:VAR}` 在 command / args / env / serverUrl / url / headers 中插值** | [官方] |

要点：
- 远程支持 stdio / Streamable HTTP / SSE 三种传输，另支持 OAuth
- **支持 `${env:VAR}` 插值** → bearer 可走「变量展开」路径，密钥不落盘（第 2 档）
- 有团队/企业 allowlist 机制：管理员一旦配置白名单，未列入的服务器会被禁用 —— 写入后若客户端不生效，需检查此项
- 仅支持 tools，不支持 prompts / resources；单次最多 100 个工具

---

## 14. Trae

| 项 | 值 | 来源 |
|---|---|---|
| 用户级路径（国际版） | `%APPDATA%\Trae\User\settings\mcp.json` | [社区] |
| 用户级路径（国内版） | `%APPDATA%\Trae CN\User\mcp.json` | [社区] |
| 项目级路径 | `<proj>/.trae/mcp.json` | [社区] |
| 格式 | JSON | [社区] |
| 根键 | `mcpServers` | [社区] |
| stdio | `{command, args[], env{}, cwd}` | [社区] |
| remote | `{url}` | [社区] |

要点：
- **国际版与国内版路径不同，且文档说法存在两种变体** → 必须按候选列表探测，并在 GUI 显示实际命中路径
- 支持 `cwd: "${workspaceFolder}"` 占位符，适合需要项目根目录的本地服务器
- 无 headers 变量展开能力（保守按字面值处理，触发 secret_policy）
- 部分配置含 `fromGalleryId` 字段（从 MCP 商店安装的标记），**写入时必须原样保留**

---

## Bearer 认证落地矩阵

目录里统一写法（见 `ARCHITECTURE.md` §4.3）：

```yaml
auth:
  type: bearer
  token_env: GITHUB_PAT
```

各客户端编码结果（14 个）：

| 客户端 | 写入形态 | 密钥落盘 |
|---|---|---|
| **Codex** | `bearer_token_env_var = "GITHUB_PAT"`（原生字段，另有 `env_http_headers = { "X-Auth" = "AUTH_ENV" }`） | 否 |
| **Claude Code** | `"headers": {"Authorization": "Bearer ${GITHUB_PAT}"}`（`${VAR}` 展开官方支持） | 否 |
| **Grok** | `[headers]` 表内 `Authorization = "Bearer ${GITHUB_PAT}"`（`${VAR}` 在 headers 中展开） | 否 |
| **Windsurf** | `"headers": {"Authorization": "Bearer ${env:GITHUB_PAT}"}`（`${env:VAR}` 在 headers/url 中插值） | 否 |
| **OpenCode** | `"headers": {"Authorization": "Bearer {env:GITHUB_PAT}"}`（**注意是 `{env:NAME}`，不是 `${VAR}`**） | 否 |
| Antigravity | headers 只认字面值 → 触发 secret_policy | 策略决定 |
| WorkBuddy | headers 只认字面值 → 触发 secret_policy | 策略决定 |
| VS Code | headers 支持但变量展开未验证 → 保守按字面值处理 | 策略决定 |
| Trae | headers 只认字面值 → 触发 secret_policy | 策略决定 |
| Cursor / Cline / Gemini CLI / Claude Desktop | 保守按字面值处理，触发 secret_policy | 策略决定 |

### 同条目多客户端混合部署时的行为

一条 `auth.token_env` 同时推给 Codex + Claude Code + Antigravity：
- Codex → 原生字段（无交互）
- Claude Code → `${VAR}`（无交互）
- Antigravity → 触发 `secret_policy`：GUI 默认弹窗一次，确认后**仅对该客户端内联**，其余仍走不落盘路径

### 读取（反向导入）语义

从现有配置扫描回收条目时，反向归一化规则：

| 现有形态 | 归一化为 |
|---|---|
| `bearer_token_env_var: "NAME"`（Codex） | `Auth::Bearer { EnvRef("NAME") }` |
| `"Bearer ${NAME}"`（Claude Code / Grok） | `Auth::Bearer { EnvRef("NAME") }` |
| `"Bearer {env:NAME}"`（OpenCode） | `Auth::Bearer { EnvRef("NAME") }` |
| `"Bearer ghp_xxxx..."`（明文） | `Auth::Bearer { Literal(...) }` + 导入告警 |
| 无 Authorization 头 | `auth: None` |

---

## 适配器实现清单

| # | 适配器 id | 需要自定义钩子 | 原因 |
|---|---|---|---|
| 1 | `workbuddy` | 否 | 标准 `mcpServers` |
| 2 | `claude-code` | 否 | 标准 `mcpServers`，需支持 `type` 字段与三层作用域 |
| 3 | `codex` | **是** | TOML + `bearer_token_env_var` / `env_http_headers` / `env_vars` |
| 4 | `kimi-code` | 否 | 标准 `mcpServers` |
| 5 | `opencode` | **是** | `command[]` 数组 + `mcp.servers` + `environment` + `disabled` + v1 结构检测 |
| 6 | `vscode` | **是** | 用户级 `servers` vs 便携级 `mcpServers` |
| 7 | `antigravity` | **是** | `serverUrl` + `timeoutSeconds` + 仅 streamable-http |
| 8 | `grok` | **是** | TOML + `startup_timeout_sec` / `tool_timeout_sec` |
| 9 | `cursor` | 否 | 标准 `mcpServers` |
| 10 | `cline` | 否 | 标准 `mcpServers` |
| 11 | `gemini-cli` | 否 | 标准 `mcpServers`（内联在 settings.json） |
| 12 | `claude-desktop` | 否 | 标准 `mcpServers`，三平台路径不同 |
| 13 | `windsurf` | **是** | `serverUrl`/`url` 双写法 + `${env:VAR}` 插值 |
| 14 | `trae` | **是** | 国际版/国内版路径探测 + `fromGalleryId` 字段保留 |

> 通用编码器覆盖 6/14，8 个需要钩子 —— 比 12 个客户端时的比例更高，说明新增客户端普遍带方言。


---

## 路径探测策略

每个适配器维护**有序候选路径列表**，Resolver 按顺序取第一个存在者（或第一个可创建者）：

```
Claude Code 用户级:
  1. $CLAUDE_CONFIG_DIR/.claude.json
  2. ~/.claude.json
```

GUI 必须显示**实际命中的路径**，而不是写死的路径 —— 实测中已发现文档与实际存在出入（如 Grok 的 `mcp.json` 与 `config.toml`）。

同时提供「自定义路径覆盖」，让未收录的新 Agent 也能用。

### 发现方式：自动侦测 + 手工指定（见 ARCHITECTURE.md §5.4.1）

- **三通道**：配置路径（权威）→ CLI 可执行文件 → 应用安装目录
- **环境变量优先**：`CLAUDE_CONFIG_DIR`、`COPILOT_HOME`、`XDG_CONFIG_HOME`、`OPENCODE_CONFIG`
- **四态**：已就绪 / 已安装未配置 / 仅配置存在 / 未检测
- **未安装也允许写入**：为 Bundle 与预配置留路
- **手工逃生舱**：路径覆盖、自定义 Agent（通用适配器）、隐藏/启用
- **边界**：侦测只回答「谁、在哪」，不擅自把现有配置合并进客户清单
