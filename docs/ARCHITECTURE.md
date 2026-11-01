# MCP Hub — 多客户端 / 多 Agent / 多环境 MCP 一键配置工具

> 状态：**架构设计稿，待确认**。本文档是开发前的唯一事实来源，确认无误后再进入实现。

---

## 1. 需求与范围

### 1.1 痛点（来自客户原话）

- MCP Server 数量多，且分散在各客户端生态里
- 客户端（Agent）种类多：Codex、WorkBuddy、OpenCode、Claude Code、Kimi Code、VS Code、Grok、Antigravity…
- 开发环境多：Windows / Linux / macOS
- **组合爆炸**：M 套 MCP × N 个 Agent × K 个环境 = 手工配置 M×N×K 次

### 1.2 目标

| 编号 | 目标          | 验收标准                                           |
| -- | ----------- | ---------------------------------------------- |
| G1 | 一键配置        | 选中若干 MCP + 若干 Agent，一次点击完成全部写入                 |
| G2 | 多客户端 | **14 个客户端**适配器，覆盖主流 CLI / IDE / 桌面端 |
| G3 | 多环境         | 同一份配置可按 Windows / Linux / macOS 差异分别落地；支持跨机器分发 |
| G4 | 图形界面        | 桌面 GUI，可视化选择、预览差异、回滚                           |
| G5 | 客户自持数据      | MCP 清单由客户维护（文件 + GUI 编辑），不依赖公共注册中心             |
| G6 | 预编译分发       | 用户下载即用，无需 Rust/Node/源码构建；编译统一在 GitHub Actions  |

### 1.3 明确不做（v0.x 范围外）

- 不做 MCP Server 的运行托管 / 进程守护（本工具只管**配置**，不管进程）
- 不做 OAuth 授权流程代理（各客户端自身的 `mcp login` 仍需手工执行）
- 不做私有 Registry 服务端
- **不做公共 MCP Registry 导入**（D4：只用客户自有清单）
- **不做 Linux 桌面 GUI**（Linux 仅提供 CLI，见 §10.1）
- **不做 SSH 远程推送**（D3：跨机器只走 Bundle 导出）
- **不做 macOS 签名与公证**（D7）
- **不做自动更新**（D8）
- **不做 OpenCode v1 兼容**（D2：仅支持 v2）

---

## 2. 现状调研结论（本机实测 + 官方文档核实）

这是本项目**最核心的技术风险**。实测发现 14 个客户端的 MCP 配置格式高度异构：

### 2.1 差异矩阵

| Agent          | 配置文件（用户级）                                        | 格式         | 根键                | 命令写法                 | 环境变量键             | 远程地址键                              |
| -------------- | ------------------------------------------------ | ---------- | ----------------- | -------------------- | ----------------- | ---------------------------------- |
| WorkBuddy      | `~/.workbuddy/mcp.json`                          | JSON       | `mcpServers`      | `command` + `args[]` | `env`             | `url`                              |
| Claude Code    | `~/.claude.json`（顶层）/ 项目 `.mcp.json`             | JSON       | `mcpServers`      | `command` + `args[]` | `env`             | `url` + `type`                     |
| Codex CLI      | `~/.codex/config.toml`                           | **TOML**   | `[mcp_servers.X]` | `command` + `args[]` | `env`             | `url` + `bearer_token_env_var`     |
| Kimi Code      | `~/.kimi-code/mcp.json`                          | JSON       | `mcpServers`      | `command` + `args[]` | `env`             | `url`                              |
| OpenCode（v2）  | `~/.config/opencode/opencode.json(c)`            | JSON/JSONC | **`mcp.servers`** | **`command[]`（数组）**  | **`environment`** | `url` + `type:remote`              |
| VS Code（用户配置）  | `%APPDATA%/Code/User/mcp.json`                   | JSON       | **`servers`**     | `command` + `args[]` | `env`             | `url` + `type`                     |
| VS Code（便携格式）  | 项目 `.mcp.json` / `~/.copilot/mcp-config.json`    | JSON       | `mcpServers`      | `command` + `args[]` | `env`             | `url` + `type`                     |
| Antigravity    | `~/.gemini/config/mcp_config.json`               | JSON       | `mcpServers`      | `command` + `args[]` | `env`             | **`serverUrl`** + `timeoutSeconds` |
| Grok           | `~/.grok/config.toml`                            | **TOML**   | `[mcp_servers.X]` | `command` + `args[]` | `env`             | `url` + `headers`                  |
| Cursor         | `~/.cursor/mcp.json`                             | JSON       | `mcpServers`      | `command` + `args[]` | `env`             | `url`                              |
| Cline          | `~/.cline/data/settings/cline_mcp_settings.json` | JSON       | `mcpServers`      | `command` + `args[]` | `env`             | `url`                              |
| Gemini CLI     | `~/.gemini/settings.json`                        | JSON       | `mcpServers`      | `command` + `args[]` | `env`             | `url`                              |
| Claude Desktop | `%APPDATA%/Claude/claude_desktop_config.json`    | JSON       | `mcpServers`      | `command` + `args[]` | `env`             | `url`                              |
| Windsurf       | `~/.codeium/windsurf/mcp_config.json`            | JSON       | `mcpServers`      | `command` + `args[]` | `env`             | `url` / `serverUrl`                |
| Trae           | `%APPDATA%/Trae/User/settings/mcp.json`（候选：`Trae CN/User/mcp.json`） | JSON | `mcpServers` | `command` + `args[]` | `env`        | `url`                              |

### 2.2 差异归纳（架构的真正驱动力）

1. **根键三种**：`mcpServers` / `servers` / `mcp.servers`
2. **文件格式三种**：JSON / JSONC（OpenCode）/ TOML（Codex、Grok）
3. **命令表达两种**：`command:"npx" + args:[]`（多数） vs `command:["npx","-y",...]`（OpenCode）
4. **环境变量键两种**：`env` vs `environment`
5. **远程地址键三种**：`url` / `serverUrl`（Antigravity、Windsurf）/ `url`+`bearer_token_env_var`（Codex）
6. **开关语义反转**：OpenCode v2 用 `disabled`（其他客户端用 `enabled` 或不写）
7. **作用域模型不同**：Claude Code 有 local/project/user 三层；Codex 有 user/project；Antigravity 有 global/workspace
8. **同一客户端存在多路径**：VS Code 用户级 `servers` vs 便携 `mcpServers`；Trae 国际版 vs 国内版路径不同

> **结论**：绝不能用「读 JSON → 改字段 → 写 JSON」或模板字符串硬拼。必须引入**规范化中间模型 + 声明式适配器**。

---

## 3. 总体架构

```
┌──────────────────────────────────────────────────────────────────┐
│  展示层（可整体替换）                                             │
│  React + TS  ── 或 Vue / Svelte / 原生 Web / Electron / 纯静态页  │
│  只依赖「命令契约」，不含任何业务逻辑                              │
└───────────────────────────┬──────────────────────────────────────┘
                            │ 命令契约（JSON 传输 + 自动生成的 TS 类型）
┌───────────────────────────▼──────────────────────────────────────┐
│  宿主适配层（薄，目标 < 200 行）                                  │
│  Tauri IPC  │  CLI 参数解析  │  HTTP / JSON-RPC（未来远程或 Web）  │
│  只做：外部调用 → 用例调用 + DTO 序列化。零业务逻辑                │
└───────────────────────────┬──────────────────────────────────────┘
                            │
┌───────────────────────────▼──────────────────────────────────────┐
│  应用服务层 mcp-app（宿主无关，Rust crate）                       │
│  用例：detect / load_catalog / plan / preview_diff / apply /      │
│        backup_list / rollback / export_bundle …                   │
│  返回纯 DTO；不依赖 Tauri、不依赖 GUI、不依赖终端                 │
└───────────────────────────┬──────────────────────────────────────┘
                            │
┌───────────────────────────▼──────────────────────────────────────┐
│  核心层  Core（Rust，纯逻辑，无 IO，可单测）                       │
│  ┌────────────┐  ┌──────────────┐  ┌──────────────────────────┐  │
│  │ Canonical  │  │  Adapter     │  │  Config IO               │  │
│  │ Model (IR) │◄─┤  Registry    │  │  JSON/JSONC/TOML 读写    │  │
│  │ 中间表示    │  │  12+ 适配器   │  │  保留未知字段 / 原子写    │  │
│  └────────────┘  └──────────────┘  └──────────────────────────┘  │
│  ┌────────────┐  ┌──────────────┐  ┌──────────────────────────┐  │
│  │ Resolver   │  │  Interpolator│  │  Backup / Diff / Audit   │  │
│  │ 路径+平台   │  │  变量插值     │  │  备份·差异·审计日志        │  │
│  └────────────┘  └──────────────┘  └──────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
                            │
┌───────────────────────────▼──────────────────────────────────────┐
│  目标层  Target                                                 │
│  Local（当前机器）  │  Bundle（导出包，跨机器）  │  SSH（v2）      │
└──────────────────────────────────────────────────────────────────┘
```

### 3.1 分层职责

| 层     | 职责              | 关键约束                        |
| ----- | --------------- | --------------------------- |
| 展示层   | 只做交互与呈现         | **可整体替换**；不含任何业务逻辑，不直接读文件   |
| 宿主适配层 | 把外部调用翻译成用例调用    | 极薄；**不写业务逻辑**，换 GUI 框架只改这一层 |
| 应用服务层 | 一次配置任务的完整生命周期   | 宿主无关；唯一持有「写文件」决策权           |
| 核心层   | 纯函数：归一化、编码、差异计算 | **无 IO / 无全局状态 / 100% 可单测** |
| 目标层   | 把「写」这个动作落到具体位置  | 本地文件 / Bundle / 远程          |

**为什么核心层要无 IO**：M×N×K 的组合爆炸必须在核心层用纯函数折叠，否则无法做穷举单测。IO 层只做「把结果落盘」。

**为什么应用服务层要独立于 Tauri**：GUI 框架是易变需求（今天 Tauri、明天可能 Electron 或 Web），业务逻辑是稳定资产。把两者隔开后，换前端不动一行 Rust 代码。

### 3.2 界面可替换性：换前端需要改什么

#### 结论

**可以整体换掉，且不需要改 Rust 侧任何业务代码。** 前提是守住三条纪律：

1. **前端不碰文件**：所有读写都经由命令契约调用 mcp-app，前端不直接访问文件系统
2. **业务代码不直接调 Tauri API**：文件选择、打开目录、通知、托盘等平台能力封装成 `platform` 接口，由宿主实现
3. **类型不手写**：命令契约的 TS 类型由 Rust 侧自动生成（`ts-rs` / `schemars`），避免两份定义漂移

#### 命令契约（唯一的耦合面）

前端只需要知道这份清单，不关心内部实现：

| 命令 | 入参 | 出参（DTO） | 状态 |
|---|---|---|---|
| `detect_all` | `{ platform, scope }` | `ClientStatus[]`（四态 + 命中路径 + 已有条目 + 支持作用域） | ✅ 已实现 |
| `load_catalog` | `{ path }` | `Catalog` + 校验告警 | ✅ |
| `plan` | `{ catalog, clients?, scope, mode, allow_comment_strip }` | `PlanOutcome`（逐文件 `FilePlan` + diff + 跳过原因） | ✅ |
| `apply` | `{ plans, dry_run }` | `ApplyReceipt`（写入数 / 备份 id / 逐文件结果） | ✅ |
| `backups` | `{}` | `BackupManifest[]` | ✅ |
| `restore` | `{ backupId }` | 恢复文件数 | ✅ |
| `save_catalog` | `{ path, catalog }` | `SaveResult` | 待实现（GUI 编辑器） |
| `export_bundle` | `{ platform, out }` | `BundleInfo` | 待实现（M6） |

> 契约一旦发布即视为稳定；新增字段向后兼容，破坏性变更走主版本号。

#### 换前端的成本对照

| 部分                        | 换前端时      | 说明                             |
| ------------------------- | --------- | ------------------------------ |
| `mcp-core`（IR / 适配器 / 编码） | **完全不动**  | 与 UI 无关                        |
| `mcp-app`（用例编排）           | **完全不动**  | 与宿主无关                          |
| `mcp-cli`                 | **完全不动**  | Linux 与 CI 依赖它                 |
| 宿主适配层                     | 重写（薄）     | Tauri IPC → 新框架的桥接，约 100–200 行 |
| `apps/web`                | 重写        | 视觉与交互想怎么改就怎么改                  |
| 平台能力（选文件/通知/托盘）           | 按新宿主要求重实现 | 已在 `platform` 接口后隔离            |

#### 视觉层为什么不影响架构

原型已经证明了这一点：那是一份**纯静态单文件 HTML + 原生 CSS**，没有框架、没有构建、没有外部依赖，却把完整交互跑通了。说明视觉与业务之间没有任何隐式绑定——换 Tailwind、换组件库、换设计系统，都只是替换渲染层。

#### 换前端的验收清单

- [ ] 新前端能完成「选清单 → 勾选目标 → 预览 diff → 应用 → 回滚」全流程
- [ ] 不新增任何写文件的代码路径（全部经 `apply_plan`）
- [ ] 复用自动生成的类型，不手写 DTO
- [ ] `mcp-cli` 的测试仍然全绿（证明 Core/服务层未被污染）

---

## 4. 核心数据模型（Canonical Model / IR）

所有适配器的**唯一**输入输出格式。IR 是与 Agent 无关的中间表示。

```rust
/// 一条 MCP 服务定义（已归一化）
pub struct McpEntry {
    pub name: String,                    // 逻辑名，在目录内唯一
    pub transport: Transport,            // Stdio | Http | Sse
    pub enabled: bool,
    pub scope: Scope,                    // User | Project | Workspace
    pub stdio: Option<StdioSpec>,
    pub http: Option<HttpSpec>,
    /// 平台差异覆盖：windows / linux / macos
    pub overrides: BTreeMap<Platform, EntryOverride>,
    /// 客户自定义备注（GUI 展示用）
    pub note: Option<String>,
}

pub enum Transport { Stdio, Http, Sse }

pub struct StdioSpec {
    pub command: String,                 // 归一化为单字符串 + args
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,   // 值支持 ${VAR} 占位
    pub cwd: Option<String>,
}

pub struct HttpSpec {
    pub url: String,
    pub headers: BTreeMap<String, HeaderValue>,
    pub auth: Option<Auth>,
    pub timeout_seconds: Option<u64>,
}

/// 请求头的值：字面量 或 环境变量引用（写入时按 Agent 能力决定策略）
pub enum HeaderValue {
    Literal(String),
    EnvRef(String),           // 引用环境变量名，永不落盘（若 Agent 支持）
}

/// 认证为一等公民字段，而非裸 headers —— 编码时按 Agent 能力映射
pub enum Auth {
    Bearer { token: SecretSource },
    // 预留：Basic { user, password } / ApiKeyHeader { header, source }
}

pub enum SecretSource {
    /// 环境变量名（推荐）：映射到 Agent 原生字段或变量展开语法，密钥不落盘
    EnvRef(String),
    /// 明文（不推荐）：目录载入即告警；写入策略默认弹窗确认
    Literal(String),
}

/// 平台差异覆盖：只写需要覆盖的字段，其余继承基线
pub struct EntryOverride {
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    pub cwd: Option<String>,
    pub url: Option<String>,
}
```

### 4.1 平台覆盖的语义

客户在 Windows 上跑，但 MCP Server 可能需要 `python3` 而非 `python`；路径分隔符、`npx.cmd` vs `npx` 也有差异。因此**每条 entry 允许带平台覆盖**：

```yaml
- name: stockdb
  transport: stdio
  command: python
  args: ["-u", "${PROJECT_ROOT}/ai_mcp/stockdb_full_mcp.py"]
  env:
    DB_HOST: localhost
  overrides:
    windows:
      command: c:/Dev/stockdata/.venv/Scripts/python.exe
    linux:
      command: /home/dev/stockdata/.venv/bin/python
```

覆盖在 **Resolver 阶段按目标平台合并**，合并后仍是 IR，之后所有适配器只看合并结果 —— **适配器不需要知道平台差异的存在**。这是保证「加平台不用改适配器」的关键。

### 4.2 变量插值

支持 `${ENV:VAR_NAME}`（读环境变量）、`${HOME}`、`${PROJECT_ROOT}`（当前工作目录）、`${HOME_DIR}`。插值在 Resolver 阶段完成，**适配器不负责插值**。

### 4.3 凭据处理（bearer 的完整语义）

客户清单里 `auth` 是一等公民字段，**推荐只写环境变量名**：

```yaml
- name: github
  transport: http
  url: https://api.githubcopilot.com/mcp/
  auth:
    type: bearer
    token_env: GITHUB_PAT      # 密钥留在环境变量里，目录可入库
    # token: ghp_xxx           # 明文也可，但载入即告警
```

#### 4.3.1 三档落地能力（编码时自动映射）


| 档 | 客户端 | 落地形态 | 密钥落盘 |
|---|---|---|---|
| 原生字段 | Codex | `bearer_token_env_var = "GITHUB_PAT"` | 否 |
| 变量展开 | Claude Code | `headers: {"Authorization": "Bearer ${GITHUB_PAT}"}` | 否 |
| | Grok | `headers.Authorization = "Bearer ${GITHUB_PAT}"` | 否 |
| | Windsurf | `"Bearer ${env:GITHUB_PAT}"`（在 headers / url 中插值） | 否 |
| | OpenCode | `headers: {"Authorization": "Bearer {env:GITHUB_PAT}"}`（注意 `{env:}` 语法） | 否 |
| 无展开能力 | Antigravity / WorkBuddy / VS Code / Cursor / Cline / Gemini CLI / Claude Desktop / Trae | headers 只认字面值，占位符会被原样发送 | **取决于策略** |

> 第三档客户端会把 `"Bearer ${GITHUB_PAT}"` 当字面字符串发给服务器，直接连不上 —— 不能盲目统一写占位符。

#### 4.3.2 无展开能力客户端的写入策略

全局 / 条目级可配置 `secret_policy`：

| 策略       | 行为                            | 默认场景                         |
| -------- | ----------------------------- | ---------------------------- |
| `ask`    | GUI 一键应用时弹窗「N 项密钥需内联，是否解析写入？」 | **GUI 默认**                   |
| `fail`   | 报错终止                          | **CLI / CI 默认**（非交互环境不能悄悄内联） |
| `inline` | 从当前环境变量解析真实值写入明文              | 用户明确接受落盘时                    |
| `skip`   | 跳过该条并标红                       | 严格模式                         |

#### 4.3.3 配套防护

- **diff 预览明文默认打码**（`Bearer ghp_****`），点开可显示
- **审计日志永不记录密钥值**，只记录「该文件写入了内联密钥」事实
- **备份是完整原文拷贝、含明文** → 备份目录权限收紧为仅当前用户（0700），设保留期（默认 30 天，可配置）
- 写入前校验：`EnvRef` 指向的环境变量未设置时标红告警；`inline` 策略下无法解析则该条降级为 `skip` 并告警

### 4.4 作用域模型（scope）

**作用域不是条目独立属性，而是「条目 × Agent」的属性** —— 各 Agent 支持的作用域不同，同一 Agent 的不同作用域还落在不同文件。

#### 支持矩阵

| Agent                                                                | user（用户级）                                      | project（项目级）                                  | local（本地级）              |
| -------------------------------------------------------------------- | ---------------------------------------------- | --------------------------------------------- | ----------------------- |
| Claude Code                                                          | ✓ `~/.claude.json` 顶层                          | ✓ `<proj>/.mcp.json`                          | ✓ `.claude.json` 内按项目嵌套 |
| Codex                                                                | ✓ `~/.codex/config.toml`                       | ✓ `<proj>/.codex/config.toml`                 | —                       |
| OpenCode（仅 v2）                                                     | ✓ `~/.config/opencode/opencode.json(c)`        | ✓ `<proj>/opencode.json(c)`                   | —                       |
| VS Code                                                              | ✓ `%APPDATA%/Code/User/mcp.json`（根键 `servers`） | ✓ `<proj>/.vscode/mcp.json`                   | —                       |
| Antigravity                                                          | ✓ `~/.gemini/config/mcp_config.json`           | ✓ `<proj>/.agents/mcp_config.json`（workspace） | —                       |
| Grok                                                                 | ✓ `~/.grok/config.toml`                        | ✓ `<proj>/.grok/config.toml`                  | —                       |
| Windsurf | ✓ `~/.codeium/windsurf/mcp_config.json` | — | — |
| Trae | ✓ `%APPDATA%/Trae/User/settings/mcp.json` | ✓ `<proj>/.trae/mcp.json` | — |
| Kimi Code / WorkBuddy / Cursor / Cline / Gemini CLI / Claude Desktop | ✓ | — | — |

#### 解析规则

```
生效作用域 = 条目.scope == inherit ? 全局默认作用域 : 条目.scope
```

- **全局默认**由 GUI 顶部「作用域」下拉 / CLI `--scope` 指定，新条目默认 `inherit`
- **条目可单独覆盖**（例如 `github` 定为项目级，随仓库共享；`stockdb` 定为用户级，全项目可用）
- 目标 Agent 不支持生效作用域时：**该组合列为跳过**，差异预览中显式提示，不静默丢弃
- 同一 Agent 的不同作用域是**不同文件**，差异预览按 `Agent × 作用域` 分组

> `local`（本地级）仅 Claude Code 支持：只对本机本项目生效，**不随仓库分发**，适合含个人密钥或本地路径的条目。

---

## 5. 适配器层设计

### 5.1 核心抽象

```rust
pub trait Adapter {
    fn id(&self) -> &'static str;             // "claude-code"
    fn display_name(&self) -> &'static str;   // "Claude Code"
    fn kind(&self) -> AdapterKind;            // Cli | Ide | Desktop
    fn spec(&self) -> &AdapterSpec;           // 声明式描述
    /// 需要非通用编码时的钩子（默认走通用引擎）
    fn encode_entry(&self, e: &McpEntry) -> Result<Value> { generic_encode(self, e) }
}
```

### 5.2 声明式 AdapterSpec

绝大部分差异可以用一张表描述，只有个别 Agent 需要自定义钩子：

```rust
pub struct AdapterSpec {
    pub format: Format,                 // Json | Jsonc | Toml
    pub root_key: Vec<String>,          // ["mcpServers"] / ["mcp","servers"] / ["mcp_servers"]
    pub entry_style: EntryStyle,
    pub env_key: &'static str,          // "env" | "environment"
    pub remote_url_key: &'static str,   // "url" | "serverUrl"
    pub type_field: Option<&'static str>,
    pub enabled_key: EnabledKey,        // None | Enabled | Disabled | InvertedEnabled
    pub supports_cwd: bool,
    pub supports_headers: bool,
    pub secret_style: SecretStyle,      // bearer 落地能力，见 4.3
    pub paths: PathTable,               // scope × os → 路径模板
}

/// bearer / 密钥引用的落地能力
pub enum SecretStyle {
    /// 原生字段引用环境变量（如 Codex 的 bearer_token_env_var）—— 密钥永不落盘
    NativeEnvField,
    /// 支持 headers 里的变量展开，携带各家语法（${VAR} / {env:VAR}）
    EnvExpansion(&'static str),
    /// headers 只认字面值 → 触发 4.3.2 的 secret_policy
    LiteralOnly,
}
```

`EntryStyle` 决定命令写法：

```rust
pub enum EntryStyle {
    CommandArgs,                        // { command: "npx", args: [...] }  ← 绝大多数
    CommandArray,                       // { command: ["npx","-y",...] }      ← OpenCode
    TomlTable,                          // TOML 的 [mcp_servers.X] 表
}
```

### 5.3 编码流程（统一入口）

```
IR Entry
  → 按 EntryStyle 选择编码器
  → stdio: command/args/env/cwd
    remote: url(/serverUrl) + headers + type
  → 注入 enabled/disabled 开关
  → 放入 AdapterSpec.root_key 指定的位置
```

### 5.4 目标客户端清单与落地路径（14 个）

路径按「作用域 × 操作系统」建表，`~` 与环境变量在 Resolver 阶段展开。

| Agent          | 作用域       | Windows                                                      | Linux                                         | macOS                                                             |
| -------------- | --------- | ------------------------------------------------------------ | --------------------------------------------- | ----------------------------------------------------------------- |
| Claude Code    | user      | `%USERPROFILE%\.claude.json`                                 | `~/.claude.json`                              | `~/.claude.json`                                                  |
| Claude Code    | project   | `<proj>\.mcp.json`                                           | 同                                             | 同                                                                 |
| Codex CLI      | user      | `%USERPROFILE%\.codex\config.toml`                           | `~/.codex/config.toml`                        | 同                                                                 |
| Codex CLI      | project   | `<proj>\.codex\config.toml`                                  | 同                                             | 同                                                                 |
| WorkBuddy      | user      | `%USERPROFILE%\.workbuddy\mcp.json`                          | `~/.workbuddy/mcp.json`                       | 同                                                                 |
| Kimi Code      | user      | `%USERPROFILE%\.kimi-code\mcp.json`                          | `~/.kimi-code/mcp.json`                       | 同                                                                 |
| OpenCode       | user      | `%USERPROFILE%\.config\opencode\opencode.json`               | `~/.config/opencode/opencode.json`            | 同                                                                 |
| OpenCode       | project   | `<proj>\opencode.json`                                       | 同                                             | 同                                                                 |
| VS Code        | user      | `%APPDATA%\Code\User\mcp.json`（根键 `servers`）                 | `~/.config/Code/User/mcp.json`                | `~/Library/Application Support/Code/User/mcp.json`                |
| VS Code        | workspace | `<proj>\.vscode\mcp.json`                                    | 同                                             | 同                                                                 |
| Antigravity    | user      | `%USERPROFILE%\.gemini\config\mcp_config.json`               | `~/.gemini/config/mcp_config.json`            | 同                                                                 |
| Antigravity    | workspace | `<proj>\.agents\mcp_config.json`                             | 同                                             | 同                                                                 |
| Grok           | user      | `%USERPROFILE%\.grok\config.toml`                            | `~/.grok/config.toml`                         | 同                                                                 |
| Cursor         | user      | `%USERPROFILE%\.cursor\mcp.json`                             | `~/.cursor/mcp.json`                          | 同                                                                 |
| Cline          | user      | `%USERPROFILE%\.cline\data\settings\cline_mcp_settings.json` | 同                                             | 同                                                                 |
| Gemini CLI     | user      | `%USERPROFILE%\.gemini\settings.json`                        | `~/.gemini/settings.json`                     | 同                                                                 |
| Claude Desktop | user      | `%APPDATA%\Claude\claude_desktop_config.json`                | `~/.config/Claude/claude_desktop_config.json` | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windsurf       | user      | `%USERPROFILE%\.codeium\windsurf\mcp_config.json`            | `~/.codeium/windsurf/mcp_config.json` | 同 |
| Trae           | user      | `%APPDATA%\Trae\User\settings\mcp.json`（候选 `Trae CN\...`） | `~/.config/Trae/User/settings/mcp.json` | `~/Library/Application Support/Trae/User/settings/mcp.json` |
| Trae           | project   | `<proj>\.trae\mcp.json` | 同 | 同 |

> 部分路径在文档与实测之间存在版本差异：Grok（`config.toml` vs `mcp.json`）、Windsurf（`~/.codeium/windsurf/mcp_config.json` vs `~/.codeium/mcp_config.json`）、**Trae（国际版 `Trae\User\settings\` vs 国内版 `Trae CN\User\`）**。**策略：每个客户端的候选路径列表按优先级探测，全部支持「自定义路径覆盖」，并在 GUI 中显示实际命中的路径。**

### 5.4.1 客户端发现：自动侦测为主 + 手工指定兜底

**结论：两者结合。** 自动侦测是默认行为，手工指定是覆盖与逃生舱。纯自动会漏（文档与实际路径有出入），纯手工则回到原始痛点。

#### 三通道探测

| 通道           | 探测内容                                                                   | 权威性 |
| ------------ | ---------------------------------------------------------------------- | --- |
| A. 配置路径      | 按候选路径列表找实际存在的配置文件                                                      | 最高  |
| B. CLI 可执行文件 | PATH 上查 `claude` / `codex` / `opencode` / `kimi` / `grok` / `gemini` / `cursor` / `code` 等 | 中   |
| C. 应用安装目录    | `%APPDATA%/Code/User`、`%APPDATA%/Claude`、`%APPDATA%/Trae`、`~/.cursor`、`~/.codeium` 等                 | 中   |

**环境变量优先于默认路径**：`CLAUDE_CONFIG_DIR`、`COPILOT_HOME`、`XDG_CONFIG_HOME`、`OPENCODE_CONFIG` 会改写配置文件位置，必须最先查。

#### 四种状态

| 状态     | 判据            | 可否勾选写入     |
| ------ | ------------- | ---------- |
| 已就绪    | 客户端存在 + 配置文件在 | ✓          |
| 已安装未配置 | 客户端在，配置文件不存在  | ✓（自动创建文件）  |
| 仅配置存在  | 配置文件在，未检测到客户端 | ✓（标注可能残留）  |
| 未检测    | 两者都无          | 默认折叠，可手工启用 |

**关键决策：未安装也允许写入配置** —— 为 Bundle 跨机器分发、新机器预配置留路。GUI 只标注状态，不阻断。

#### 手工指定三个逃生舱

1. **路径覆盖**：每 Agent × 每作用域可改，覆盖自动探测结果
2. **自定义 Agent**：未收录的客户端用通用适配器手工声明（路径 + 根键 + 命令写法 + 变量键），无需等官方适配
3. **隐藏/启用**：手动排除不需要的客户端，控制客户端列数

#### 边界：侦测只回答「谁、在哪」，不回答「配什么」

自动侦测**不会**把现有配置擅自合并进客户清单。读取现有配置仅用于：

- 客户端配置页状态显示（已配置 / 有差异）
- 用户**显式触发**的反向导入（`ARCHITECTURE.md` §4.3 的归一化规则）

配哪些 MCP 永远由客户清单决定。

### 5.5 需要自定义钩子的客户端

| Agent       | 原因                                                                                                |
| ----------- | ------------------------------------------------------------------------------------------------- |
| OpenCode | **仅支持 v2**：`command` 数组 + `mcp.servers` 根键 + `environment` + `disabled`（语义反转）。检测到 v1 扁平结构时提示迁移，不按 v1 写入 |
| Codex       | TOML + `bearer_token_env_var` / `env_http_headers` / `env_vars` 等 Codex 专有字段                      |
| Antigravity | 远程地址键是 `serverUrl`，不支持 `url`；另有 `timeoutSeconds`                                                  |
| VS Code     | 同一 Agent 两套根键（用户级 `servers` vs 便携级 `mcpServers`），按目标路径选择                                          |
| Grok        | TOML + `startup_timeout_sec` / `tool_timeout_sec`                                                 |
| Windsurf | 远程可用 `serverUrl` 或 `url`；`${env:VAR}` 在 command/args/env/serverUrl/url/headers 中插值 |
| Trae | 国际版/国内版路径不同需探测；支持 `cwd: "${workspaceFolder}"` 占位 |

---

## 6. 配置写入引擎（安全第一）

这是最容易造成用户损失的地方 —— 一旦写坏 `~/.claude.json` 或 `~/.codex/config.toml`，用户对应 Agent 直接不可用。

### 6.1 铁律

1. **绝不覆盖未知字段**：读入 → 只修改目标子树 → 其余原样写回
   - JSON/JSONC：`serde_json` + `preserve_order` 特性保序
   - TOML：`toml_edit`（保留注释与格式，Codex 的 `config.toml` 有大量注释与 `[plugins.*]` 等无关段）
2. **先备份后写**：每次写入前在 `~/.mcp-hub/backups/<agent>/<timestamp>/` 留一份原文
3. **原子写**：写临时文件 → fsync → rename，避免半截文件
4. **先预览后执行**：GUI 展示逐文件 diff，用户确认才落盘
5. **写后校验**：重新解析目标文件，解析失败立即自动回滚
6. **全程审计**：每次变更写入 `~/.mcp-hub/audit.log`

### 6.2 写入算法

```
for (target_file, entries) in plan:
    backup(target_file)                          # 1
    doc = parse_preserving(target_file)          # 2  解析失败 → 中止该文件，绝不重建
    root = doc.navigate(root_key)                # 3  子树不存在则创建，同级其他键不动
    for entry in entries:
        existing = root.get(entry.name)
        if existing is None:
            root.set(entry.name, encode(entry))            # 新增
        elif merge(existing, encode(entry)) != existing:
            root.set(entry.name, merge(existing, encode(entry)))  # 字段级合并
        else:
            skip                                            # 幂等：无变化则不写
    atomic_write(target_file, doc)               # 5
    if parse(target_file) fails: rollback()       # 6
```

### 6.3 删除语义

「未勾选」不等于「删除」。删除是**显式操作**，GUI 提供独立勾选，且删除前必须过差异预览。

### 6.4 冲突处理

同一 Agent 同一 scope 下同名条目：

- 目录版本 vs 现有配置 → 以**客户目录为准**（目录即事实来源），但差异在 GUI 高亮
- 目录内重复名 → 载入即报错，拒绝启动写入

### 6.5 覆盖语义：不会整文件覆盖，同名条目按字段合并

这是用户最关心的问题 —— **"我已经在 WorkBuddy 里配好了，工具再写会不会把原来的覆盖掉？"**

答案分三层，粒度从粗到细：

| 层级      | 行为                       | 说明                               |
| ------- | ------------------------ | -------------------------------- |
| **文件级** | **绝不整文件覆盖**              | 读原文 → 只改目标子树 → 写回；文件里的其他所有内容原样保留 |
| **条目级** | 非同名条目**完全不动**            | 你自己加的其它 MCP 配置原样保留，不会被"清理"       |
| **字段级** | 同名条目**按字段合并**，保留清单未声明的字段 | 默认 `merge`；可切 `replace` 整条替换     |

#### 三种同名场景

| 场景                 | 行为                     | GUI 标注          |
| ------------------ | ---------------------- | --------------- |
| 文件里没有该条目           | 新增                     | `新增`            |
| 文件里有，内容与清单**完全一致** | **跳过，不写入**（幂等，不产生备份噪音） | 不列出             |
| 文件里有，但内容不同         | 字段级合并后写入               | `修改` + 逐字段 diff |
| 文件里有，清单里没有         | **保留**                 | 不列出             |

#### 字段级合并规则（默认 `merge`）

- 清单**声明了**的字段 → 以清单为准覆盖
- 清单**没声明**的字段 → **保留文件里的原值**（例如你手工加的 `timeout`、`cwd`、额外 `headers`）
- 需要"删掉某个字段" → 切 `replace` 模式整条替换，diff 会显示被删除的行，确认后才执行

#### 具体示例：WorkBuddy 已有配置

写入前 `~/.workbuddy/mcp.json`：

```json
{
  "mcpServers": {
    "stockdb-native": {
      "command": "c:/Dev/stockdata/.venv/Scripts/python.exe",
      "args": ["-u", "c:/Dev/stockdata/ai_mcp/stockdb_full_mcp.py"],
      "cwd": "c:/Dev/stockdata/ai_mcp"
    },
    "my-private-tool": { "command": "node", "args": ["private.js"] }
  }
}
```

清单里有 `stockdb`，没有 `stockdb-native`、`my-private-tool`。写入后：

```json
{
  "mcpServers": {
    "stockdb-native": { "...完全原样保留..." },
    "my-private-tool": { "...完全原样保留..." },
    "stockdb": { "command": "python", "args": ["-u", "..."] }
  }
}
```

- `stockdb`：**新增**
- `stockdb-native`、`my-private-tool`：**一个字都没动**

若清单里恰好也叫 `stockdb-native` 且只声明了 `command` + `args`：


- `command` / `args` → 被清单值更新
- `cwd` → **保留**（清单没声明）

#### 保护机制

1. **解析失败即中止**：目标文件不是合法 JSON/TOML（例如被手工改坏、或有 BOM/注释不兼容）→ 该文件**直接跳过并报错**，绝不"重新生成一份"覆盖它
2. **写前必备份**：原文完整存到 `~/.mcp-hub/backups/<agent>/<timestamp>/`，可一键回滚
3. **写后必校验**：重新解析，失败自动回滚到备份
4. **不注入私有字段**：不往你的配置文件里塞 `_source: mcp-hub` 之类标记（可能被客户端拒绝），所有权记录放在工具自己的 `~/.mcp-hub/manifest.json`（v0.2）
5. **幂等**：重复应用同一份清单，第二次不会产生任何写入或备份

---

## 7. 客户数据供给（Catalog）

客户自行维护的 MCP 清单，是整个系统的**事实来源**。格式：**YAML 为主，JSON 兼容**（JSON 是 MCP 生态通用格式，兼容导入）。

```yaml
version: 1
entries:
  - name: stockdb
    description: 本地行情数据库
    transport: stdio
    command: python
    args: ["-u", "${PROJECT_ROOT}/ai_mcp/stockdb_full_mcp.py"]
    env:
      STOCKDB_URL: ${ENV:STOCKDB_URL}
    overrides:
      windows:
        command: c:/Dev/stockdata/.venv/Scripts/python.exe
    tags: [finance, local]

  - name: context7
    description: 文档检索
    transport: http
    url: https://mcp.context7.com/mcp
    auth:
      type: bearer
      token_env: CONTEXT7_API_KEY      # 密钥留在环境变量，按 Agent 能力自动映射
```

### 7.1 数据流

```
客户 YAML/JSON  ──解析──▶ 校验（schema + 重名 + 平台覆盖合法性）
                              │
                     Canonical Catalog (IR)
                              │
              ┌───────────────┴───────────────┐
              ▼                               ▼
      客户端配置视图                     导出 Bundle（跨机器）
```

### 7.2 只做客户自有清单（已确认）

**不接公共 MCP Registry。** 清单唯一来源是客户自己维护的 YAML/JSON 文件，由客户自行版本管理、自行评审。理由：

- 客户明确要求「MCP 配置数据由客户提供」
- 公共 Registry 处于 preview，无可用性承诺，数据可能被投毒或删除
- 少一个外部依赖 = 少一个故障点与合规面

因此 `mcp-app` **不含任何网络同步逻辑**，工具完全离线可用（除用户自己的 MCP 服务器本身需要联网）。

---

## 8. GUI 设计

### 8.1 主界面：客户端配置

客户端配置页是整个工具的心脏 —— 一屏覆盖 M×N 痛点。

```
┌──────────────────────────────────────────────────────────────────────┐
│  MCP Hub        目录: customer-mcp.yaml ▾   目标: [本机] ▾   [检测]  │
├──────────────────────────────────────────────────────────────────────┤
│  MCP 服务            │ Claude │ Codex │ Kimi │ OpenC │ VSCode │ …    │
│  ────────────────────┼────────┼───────┼──────┼───────┼────────┼───    │
│  ☑ stockdb      本地  │   ●    │   ●   │  ●   │   ●   │   ●    │      │
│  ☑ context7   远程    │   ○    │   ○   │  ○   │   ○   │   ○    │      │
│  ☐ github       远程  │   ○    │   ◐   │  ○   │   ○   │   ○    │      │
│  ────────────────────┴────────┴───────┴──────┴───────┴────────┴───    │
│  ● 已配置   ○ 未配置   ◐ 有差异                                          │
├──────────────────────────────────────────────────────────────────────┤
│  已选 3 个 MCP × 6 个 Agent = 18 项                        [一键应用] │
└──────────────────────────────────────────────────────────────────────┘
```

### 8.2 功能清单

| 页面        | 能力                                                                    |
| --------- | --------------------------------------------------------------------- |
| **客户端配置** | 主界面。行列勾选、状态着色、批量应用                                                    |
| **目录**    | 导入/导出 YAML/JSON、条目编辑、平台覆盖编辑、标签、校验                                     |
| **差异预览**  | 应用前逐文件展示 diff（新增/修改/删除），逐条勾选是否应用                                      |
| **备份与回滚** | 按 Agent 查看历史备份，一键回滚到任意时间点                                             |
| **环境检测**  | 三通道自动侦测（§5.4.1），显示四态（已就绪/已安装未配置/仅配置存在/未检测）、实际命中的配置路径、该 Agent 已配置的 MCP |
| **设置**    | 自定义路径覆盖、插值变量、审计日志、默认作用域                                               |

### 8.3 技术栈

| 层    | 选型                        | 理由                                                                           |
| ---- | ------------------------- | ---------------------------------------------------------------------------- |
| 桌面框架 | **Tauri v2**              | 单二进制 <10MB（Electron ~150MB）；复用 Rust 核心；**GUI 仅 Windows / macOS，Linux 走 CLI** |
| 前端   | React + TypeScript + Vite | 类型安全、组件生态成熟                                                                  |
| 后端   | Rust（Orchestrator + Core） | `toml_edit` / `serde_json` 是本项目的核心依赖，Rust 生态最成熟                              |

**本机可行性已实测**：Rust 1.85 + MSVC 14.44 + crates.io 可达 + WebView2 155 已安装 → Tauri v2 可构建可运行。

### 8.4 视觉方案（D6：唯一要求是漂亮）

技术栈不设限，但**"漂亮"要落到可执行的设计规范上**，否则实现阶段会各写各的。

#### 组件与工具选型

| 项 | 选择 | 理由 |
|---|---|---|
| 样式方案 | **手写 CSS + 设计令牌**（v0.1 实测决定） | 零组件库依赖、构建 165ms、无供应链与版本风险；视觉完全可控 |
| 组件 | 自建轻量原语（Panel / Pill / Banner / Field） | 只做需要的，不引入黑盒 |
| 图标 | 内联 SVG / 符号 | 避免为几个图标引入整包依赖 |
| 动效 | CSS transition（≤120ms） | 只做 background / opacity / border；不引入动画库 |
| 字体 | 系统 UI 栈 + 等宽栈 | 中文用「苹方 / 微软雅黑 / Noto Sans SC」；等宽用 `ui-monospace` |

> 曾计划引入 Tailwind + shadcn/ui，实测后放弃：本项目 UI 面窄（5 屏），手写 CSS 更小更稳，
> 且设计令牌已固化（见下），后续换任何方案都不影响信息架构。

#### 设计 tokens

```
色板：中性灰阶（背景/边框/文字三级） + 单一强调色 + 4 个语义色
强调色：#534AB7（靛蓝，专业感，不轻浮）
语义色：成功 #1D9E75 / 警告 #BA7517 / 危险 #A32D2D / 信息 #185FA5
圆角：8（控件）/ 12（卡片）/ 16（弹层）
描边：0.5px，弱对比
阴影：仅浮层使用，极浅；卡片不用阴影改用描边
字号：11（辅助）/ 12（正文）/ 13（强调）/ 15（页面标题）
密度：表格行高 36px，紧凑模式 32px
主题：亮 / 暗双主题，CSS 变量驱动，跟随系统
```

#### 美观的三条执行原则

1. **靠留白和层次，不靠装饰**：卡片用 0.5px 描边而非阴影；分隔用细线而非色块
2. **色彩语义化**：颜色只用来表达状态（已配置 / 有差异 / 不支持 / 风险），不做装饰性配色
3. **克制动效**：hover 与选中态用 120ms 过渡；列表展开、页面切换不做花哨动画

#### 14 列矩阵的排版策略（D1 的直接后果）

14 个客户端全上，矩阵必然超宽，这是必须解决的设计问题：

- **横向滚动 + 首列冻结**（MCP 配置名称列始终可见）
- **列头紧凑化**：图标 + 短名（3–4 字），完整名称放 tooltip
- **按类型分组**：CLI / IDE / Desktop 三组，组间加分隔与组标题
- **列可隐藏**：只用 5 个客户端的用户，可把其余收起来
- **密度切换**：标准 / 紧凑两档
- **批量操作入口**：整列全选（某客户端全配）、整行全选（某 MCP 全推）

> 若 14 列体验仍不理想，备选方案是「按客户端分组的分页视图」——但优先尝试横向滚动，因为对比是矩阵的核心价值。

#### 运行与评审方式

| 目的 | 命令 |
|---|---|
| 只评审 UI（浏览器，含演示数据） | `cd apps/web && npm run dev` → 打开 `http://localhost:5173/?mock=1` |
| 深链某屏 | `?mock=1&page=matrix\|catalog\|diff\|detect\|backups` |
| 桌面端开发 | `npm --prefix apps/web run tauri dev`（需 rustc ≥ 1.90） |
| 桌面端构建 | `npm --prefix apps/web run tauri build`（由 CI 执行） |

前端内置 **mock 回退**：没有 Tauri 运行时时自动使用演示数据，
因此 UI 可以脱离桌面壳独立评审与迭代 —— 这也验证了展示层与业务层确实解耦。

> 桌面壳要求 rustc ≥ 1.90（Tauri 2.10 的 MSRV）。核心四层（core/io/app/cli）仍按 1.85 构建，
> 因此 `default-members` 不含 `mcp-desktop`，默认 `cargo build` 不受影响。

#### 关键界面状态

每个页面必须实现四态，不能只有一个 happy path：

| 状态 | 表现 |
|---|---|
| 空 | 插画/图标 + 一句引导 + 主操作按钮（例：还没选目标 → "去勾选"） |
| 加载 | 骨架屏，不用 spinner 转圈 |
| 错误 | 错误原因 + 可执行动作（重试 / 打开日志 / 回滚）|
| 危险确认 | 破坏性操作二次确认，展示影响范围与回滚方式 |


---

## 9. 工程结构

```
C:/Dev/mcpconfig/
├─ Cargo.toml                     # workspace
├─ crates/
│  ├─ mcp-core/                   # ① 纯逻辑：IR、解析、插值、差异、校验（无 IO、无 UI 依赖）
│  │  ├─ src/
│  │  │  ├─ model.rs              # Canonical Model
│  │  │  ├─ catalog.rs            # YAML/JSON 清单解析 + 校验
│  │  │  ├─ interpolate.rs        # ${ENV:VAR} 插值
│  │  │  ├─ resolve.rs            # 平台覆盖合并 + 路径解析
│  │  │  ├─ diff.rs               # 条目级差异
│  │  │  └─ adapters/
│  │  │     ├─ mod.rs             # Adapter trait + AdapterSpec
│  │  │     ├─ generic.rs         # 通用编码器
│  │  │     ├─ claude_code.rs     # … 每个客户端一个文件
│  │  │     ├─ codex.rs
│  │  │     ├─ opencode.rs
│  │  │     ├─ antigravity.rs
│  │  │     └─ ...
│  ├─ mcp-io/                     # ② 写入引擎：备份 / 原子写 / 校验回滚 / 审计（唯一碰文件的地方）
│  ├─ mcp-app/                    # ③ 应用服务层：用例编排，宿主无关
│  │  ├─ src/
│  │  │  ├─ commands.rs           # 命令契约：detect/load_catalog/plan_apply/preview_diff/apply/rollback…
│  │  │  ├─ dto.rs                # 纯数据返回类型（serde + ts-rs 导出 TS）
│  │  │  └─ platform.rs           # 平台能力接口（选文件/打开目录/通知），由宿主实现
│  │  └─ bindings/                # 构建时生成的 TypeScript 类型（前端 import）
│  ├─ mcp-cli/                    # ④ 宿主 A：CLI（Linux / CI / 服务器）
│  └─ mcp-desktop/                # ④ 宿主 B：Tauri v2（薄适配 + 窗口）
├─ .github/
│  └─ workflows/
│     ├─ ci.yml                   # PR 检查：fmt/clippy/test/前端构建
│     └─ release.yml              # Tag 发布：GUI 两平台 + CLI 三平台 + Release 产物
├─ apps/
│  └─ web/                        # ⑤ 展示层：React + TS + 手写 CSS（含 mock 回退，可独立评审）
├─ docs/
│  ├─ ARCHITECTURE.md             # 本文档
│  ├─ adapters.md                 # 各客户端适配器细则与实测样例
│  └─ prototype/index.html        # 交互原型（纯静态，可独立运行）
└─ examples/
   └─ customer-mcp.yaml           # 示例客户清单
```

**依赖方向是单向的**（违反即为架构错误，CI 应加检查）：

```
apps/web ──► 命令契约 ──► mcp-desktop / mcp-cli ──► mcp-app ──► mcp-io ──► mcp-core
```

`mcp-core` / `mcp-io` / `mcp-app` **不得**依赖 `tauri` / `clap` / 任何 GUI 或 CLI 库。这条约束可以用 `cargo tree` 断言。

---

## 10. 分发与 CI/CD（预编译分发，不做源码部署）

**原则：最终用户拿到的是预编译产物，不需要安装 Rust / Node，不需要 clone 源码构建。所有编译在 GitHub Actions 完成。**

选择 GitHub Actions 构建的额外收益：**macOS 安装包必须在 mac 环境构建**，Windows 本机无法产出 `.dmg`，CI 矩阵是唯一省事路径。

### 10.1 交付物形态（按平台）

| 平台        | GUI                             | CLI                                 | 说明                           |
| --------- | ------------------------------- | ----------------------------------- | ---------------------------- |
| Windows   | `.msi` / `.exe`(NSIS) 安装包 + 便携版 | `mcp-hub-cli.exe`                   | Win11 自带 WebView2；缺失时安装器引导安装 |
| macOS     | `.dmg`                          | `mcp-hub-cli`（apple-darwin）         | 签名/公证见 §10.3                 |
| **Linux** | **不提供**                         | **`mcp-hub-cli`（tar.gz，可选 `.deb`）** | **客户明确：Linux 仅命令行，不要求 GUI**  |

**Linux 不做 GUI 是范围简化而非降级**：服务器与容器环境本就无图形界面，CLI 跑同一份客户清单、共用 Core，编码与写入行为与 GUI 完全一致。

由此带来的直接收益：

- **不需要 `webkit2gtk-4.1` 运行库**，也没有 AppImage 发行版兼容性问题
- Linux CI 只需 `cargo build --release -p mcp-cli`，不跑 Tauri 打包，构建更快更稳
- Linux 产物用 **`x86_64-unknown-linux-musl` 静态构建**，做到真正的单文件免依赖（默认 gnu 目标会动态链接 glibc，老服务器可能跑不起来），`scp` 到服务器即可用，配合 Bundle 跨机器分发刚好闭环

### 10.2 两条流水线

#### `ci.yml`（PR / push 到 main）

| 任务 | 内容 |
|---|---|
| `rust` | `cargo fmt --check`、`clippy -D warnings`、`cargo test`（默认成员：core/io/app/cli） |
| `arch-guard` | 用 `cargo tree` 断言 `mcp-core` / `mcp-app` 不依赖 `tauri` / `clap` —— 架构约束靠 CI 守，不靠自觉 |
| `web` | `npm ci` + `npm run build`（含 `tsc --noEmit`） |
| `desktop-check` | Windows 上 `npm ci && npm run build` 后 `cargo check -p mcp-desktop`（Tauri 编译期需要 dist） |

#### `release.yml`（push tag `v*`）

| 任务 | 内容 |
|---|---|
| `create-release` | 先建 Release，避免多个构建任务并发创建同一 Release |
| `cli`（矩阵 3 个） | `linux-x64`（`x86_64-unknown-linux-musl` 静态）、`windows-x64`、`macos-arm64`；打包为 `.tar.gz` / `.zip` |
| `gui`（矩阵 2 个） | `windows-latest` / `macos-latest`，`tauri-action` 打包并上传（**未签名**） |
| `checksums` | 下载全部产物，生成 `SHA256SUMS` 并上传 |

产物命名：`mcp-hub-v0.1.0-linux-x64.tar.gz`、`mcp-hub-v0.1.0-windows-x64.zip`、`mcp-hub-v0.1.0-macos-arm64.tar.gz`；
GUI 为 `MCP Hub_0.1.0_x64-setup.exe` / `MCP Hub_0.1.0_aarch64.dmg`（Tauri 默认命名）。
v0.4.3 起名称统一由 `scripts/publish_release.py` 的 `cli_name` / `gui_installer_name` 生成：CLI 为 `mcp-hub-cli-v<版本>-<平台>`，
macOS GUI 为 `MCP.Hub_<版本>_macos-arm64.dmg`，并在 CI 中校验 DMG 内 `.app` 的 ad-hoc 签名（`bundle.macOS.signingIdentity = "-"`）。

#### 发布流程

```bash
git tag v0.1.0
git push origin v0.1.0     # 触发 release.yml
```

> 本机 `github.com` 直连不可用（实测 000），push 需走代理；**Actions 端网络正常**。

### 10.3 两个必须提前说明的坑

| 坑 | 影响 | 对策 |
|---|---|---|
| **macOS 不签名（已确认）** | 首次打开 `.dmg` 会触发 Gatekeeper 提示 | **不做签名与公证**。README 给出两步绕过：右键 →「打开」；或 `xattr -cr "/Applications/MCP Hub.app"`。下载页注明「本构建未签名」 |
| **Windows WebView2** | 老系统可能缺失 | Win11 自带；Tauri 安装器配置为缺失时自动引导安装（offline bootstrapper） |

> ~~Linux webkit2gtk 运行库依赖~~ 已随「Linux 仅 CLI」决策彻底规避。

> macOS 不签名是**明确的产品决策**，不是疏忽：省掉 $99/年开发者账号与公证流程。代价是用户首次打开多两步操作，必须在文档写清楚，否则会被当成安装失败。

### 10.4 发布与更新策略

- **可复现构建**：`Cargo.lock` / `package-lock.json` 入库，`rust-toolchain.toml` 固定工具链版本
- **产物校验**：Release 附 `SHA256SUMS`，文档给出校验命令
- **不做自动更新（已确认）**：不接入 Tauri updater，不检查新版本、不联网拉取更新包。升级 = 用户自行下载新包覆盖安装。好处是零后台网络行为、无需维护签名密钥对
- **本机网络注意**：实测 `github.com` 直连 000，本机 `git push` 需走代理；**GitHub Actions 端网络正常，不受影响**

---

## 11. 里程碑

| 里程碑    | 内容          | 交付物                                                                                |
| ------ | ----------- | ---------------------------------------------------------------------------------- |
| **M0** | 架构确认        | 本文档 + 适配器矩阵确认                                                                      |
| **M1** | Core 骨架 | ✅ IR、清单解析、插值、平台覆盖合并、单测全绿 |
| **M2** | 适配器全量 | ✅ 14 个客户端适配器 + 往返（round-trip）测试 |
| **M3** | 写入引擎 | ✅ 备份 / 原子写 / 字段级合并 / diff / 回滚 / 审计（`mcp-io`） |
| **M4** | 应用服务层 + CLI | ✅ `mcp-app` 用例层 + `mcp-hub` CLI（doctor/diff/apply/backups/restore） |
| **M5** | GUI | ✅ 前端 5 屏 + Tauri 薄适配；桌面壳编译需 rustc ≥ 1.90（CI 验证） |
| **M6** | 跨机器 | Bundle 导出（YAML + 平台变体）供另一台机器导入；**不做 SSH 远程推送** |
| **M7** | 发布管线 | ✅ `.github/workflows/{ci,release}.yml`：PR 四任务 + tag 触发三平台 CLI 与两平台 GUI |

---

## 12. 风险与对策

| 风险 | 等级 | 对策 |
|---|---|---|
| 客户端配置格式随版本漂移 | **高** | 适配器按目标客户端当前格式写；文档标注适用范围；提供「自定义路径 + 原始 JSON 覆盖」逃生舱。OpenCode 仅支持 v2，检测到 v1 结构时提示迁移而非静默改写 |
| 写坏用户配置文件导致客户端不可用 | **高** | 备份 + 原子写 + 字段级合并 + 保留未知字段 + 写后校验 + 自动回滚 + 强制 diff 预览 |
| 密钥泄露进配置文件 | 中 | 清单只存环境变量名；写入前扫描并告警明文；优先映射原生 env 引用字段；内联需显式策略 |
| 14 个客户端配置格式持续演进 | 中 | 适配器声明式描述 + 每个客户端一份真实样本做 round-trip 测试；新增格式只加声明不改引擎 |
| macOS 未签名被 Gatekeeper 拦截 | 中 | 已确认不签名；README 与下载页明确绕过步骤（右键打开 / `xattr -cr`），避免被误判为安装失败 |
| 各客户端实际路径与文档不符 | 中 | 候选路径列表探测（Trae 国际版/国内版、Windsurf 双路径、Grok 双格式）+ GUI 显示实际命中路径 + 自定义覆盖 |
| 前端框架锁定 / 改版成本高 | 低 | 业务逻辑全在 `mcp-app`，前端只依赖命令契约；换前端仅重写宿主适配层与 `apps/web`（见 §3.2） |
| Tauri 跨平台构建 | 低 | GUI 仅 Windows/macOS；Linux 只构建 CLI（纯 Rust，无 GUI 依赖） |
| GitHub 网络受限（实测 000） | 低 | 编译在 GitHub Actions（网络正常）；本机 `git push` 走代理；crates.io / npm registry 实测可达 |

---

## 13. 已确认决策（客户拍板，2026-10-26）

| # | 决策 | 落地影响 |
|---|---|---|
| D1 | **14 个客户端全上** | 适配器范围：Claude Code、Codex、WorkBuddy、Kimi Code、OpenCode(v2)、VS Code、Antigravity、Grok、Cursor、Cline、Gemini CLI、Claude Desktop、**Windsurf**、**Trae** |
| D2 | **OpenCode 只支持 v2** | 根键固定 `mcp.servers`、`command[]` 数组、`environment`、`disabled`；不做 v1 兼容，不做版本探测；检测到 v1 扁平结构时**提示迁移**，不静默改写 |
| D3 | **只导出配置** | 目标层仅 `Local` + `Bundle`；**移除 SSH 远程推送** |
| D4 | **只做客户自有清单** | 不接公共 MCP Registry；`mcp-app` 不含任何网络同步逻辑，完全离线可用 |
| D5 | **默认全局（用户级）** | 全局默认作用域 = `user`；条目可逐条覆盖为 `project` / `local` |
| D6 | **GUI 只要漂亮，技术栈不设限** | 采用 Tauri v2 + React + TS；视觉方案见 §8.4（设计系统与组件选型） |
| D7 | **macOS 不签名** | 不做签名与公证；README 与下载页给出绕过步骤 |
| D8 | **不做自动更新** | 不接 Tauri updater，无后台版本检查；升级靠用户下载新包覆盖安装 |

### 由此产生的两条硬约束

1. **D2 使 OpenCode 适配器简化**：去掉 v1/v2 双写与版本探测分支，但必须保留「v1 结构检测 + 迁移提示」，否则老用户会被静默写坏
2. **D4 使依赖面收敛**：`mcp-app` 不得引入 HTTP 客户端依赖做注册中心同步；唯一的网络行为来自用户自己的 MCP 服务器

---

## 14. 待细化（不阻塞开发，实现中定）

1. **GUI 视觉细节**：设计 tokens 已定（§8.4），具体页面排版在实现中迭代
2. **Bundle 包格式**：zip 还是 tar.gz、是否含 README 与校验清单
3. **Trae 国内版路径优先级**：`Trae CN\User\mcp.json` 与 `Trae\User\settings\mcp.json` 的探测顺序需在真机确认
4. **备份保留期**：默认 30 天是否合适
