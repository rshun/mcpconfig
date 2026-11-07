# MCP Hub

多客户端 / 多环境 MCP 配置一键分发工具。

把一份客户自维护的清单，按各客户端的原生格式写入 14 个 MCP 客户端——不用手工配 M×N×K 次。

当前版本：**v0.4.4**。本版导入时识别到的凭据默认原样复制到配置库，也可逐个字段改为环境变量映射；扫描和预览中凭据保持隐藏。使用及回退见 [发布说明](docs/releases/v0.4.4.md)。

## 现在的状态

| 模块 | 状态 |
|---|---|
| `mcp-core`：IR、清单解析、插值、平台覆盖、14 个适配器 | ✅ 实现，69 项测试 |
| `mcp-io`：备份 / 原子写 / 写后校验 / 回滚 / 审计 | ✅ 实现，21 项测试 |
| `mcp-app` 应用服务层（检测 / 计划 / 应用 / 备份 / 软件设置） | ✅ 实现，包含持久化与项目隔离回归测试 |
| `mcp-hub` CLI（doctor / diff / apply / backups / restore） | ✅ 实现，可执行 |
| GUI 前端（React + TS，配置库优先） | ✅ 配置库、Agent 导入、任务对话框，独立全局与项目范围，持久化软件设置 |
| GUI 桌面壳（Tauri v2 薄适配） | ✅ Windows / macOS 安装包，由 GitHub Actions 验证与构建 |

设计文档见 [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)，客户端方言细则见 [`docs/adapters.md`](docs/adapters.md)，
可交互原型见 [`docs/prototype/index.html`](docs/prototype/index.html)。

## 支持的客户端（14）

写入安全修复及其边界见 [安全修复说明](docs/SAFETY.md)。
界面设计和后续方向见 [设计说明](docs/UI-DIRECTION.md)，可离线打开 [交互原型](docs/ui-workbench.html)。
正式 GUI 从 v0.4.0 起采用配置库优先布局。原型只演示操作，不读取或修改本机配置。

Claude Code · Codex CLI · WorkBuddy · Kimi Code · OpenCode(v2) · VS Code · Antigravity · Grok ·
Cursor · Cline · Gemini CLI · Claude Desktop · Windsurf · Trae

## 快速验证

```bash
# 跑测试
cargo test -p mcp-core

# 看同一份配置在 14 个客户端下的落地形态
cargo run -p mcp-core --example matrix_dump

# 端到端演示：清单 → 计划 → diff → 写入 → 回滚（全程在临时目录，不碰真实配置）
cargo run -p mcp-io --example end_to_end
```

`matrix_dump` 会打印出「同一条 github 配置」在 14 家客户端里的不同写法，例如：

| 客户端 | bearer 落地 |
|---|---|
| Codex | `bearer_token_env_var = "GITHUB_PAT"` |
| Claude Code / Grok | `"Bearer ${GITHUB_PAT}"` |
| Windsurf | `"Bearer ${env:GITHUB_PAT}"` |
| OpenCode | `"Bearer {env:GITHUB_PAT}"` |
| WorkBuddy / VS Code / Cursor… | `"Bearer <需内联:GITHUB_PAT>"`（需按策略处理） |

## 图形界面

Windows 用户可下载 [GUI 安装程序](https://github.com/rshun/mcpconfig/releases/download/v0.4.4/MCP.Hub_0.4.4_x64-setup.exe)，
或 [GUI 免安装 ZIP](https://github.com/rshun/mcpconfig/releases/download/v0.4.4/MCP.Hub_0.4.4_windows-x64-portable.zip)。
ZIP 完整解压后运行 `MCP.Hub.exe`；它使用系统已有的 [Microsoft WebView2 Runtime](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)，缺少时需先安装运行时。
配置与备份仍使用用户目录，详见 [免安装说明](docs/WINDOWS-PORTABLE.txt)。`mcp-hub-cli-v*-windows-x64.zip` 是独立 CLI 包。

macOS（Apple Silicon）用户可下载 [GUI 安装包](https://github.com/rshun/mcpconfig/releases/download/v0.4.4/MCP.Hub_0.4.4_macos-arm64.dmg)，即 `MCP.Hub_<版本>_macos-arm64.dmg`（v0.4.2 及更早为 `MCP.Hub_<版本>_aarch64.dmg`），
打开后把 **MCP Hub** 拖入「应用程序」。`mcp-hub-cli-*` / `mcp-hub-v*-macos-arm64.tar.gz` 是命令行包，解压后双击会打开终端。
从 v0.4.3 起 macOS 应用使用 ad-hoc 签名（非 Apple 开发者证书、未公证），首次打开会提示“无法验证开发者”：
点「完成」后到「系统设置 → 隐私与安全性」点「仍要打开」，之后可直接启动，无需执行 `xattr`。


```bash
cd apps/web
npm install
npm run dev        # 打开 http://localhost:5173/?mock=1
```

浏览器里带 `?mock=1` 会用内置演示数据渲染（不需要后端），适合评审 UI。
桌面端：

```bash
npm --prefix apps/web run tauri dev     # 需 rustc >= 1.90
```

默认进入「我的 MCP」，侧栏只有「我的 MCP / Agents / 变更历史」，底部独立提供「软件设置」。
所有配置都保留在库中；不适用当前范围的条目会说明原因，不会消失。添加、编辑、目录选择、导入和预览通过独立任务对话框完成。
批量矩阵保留为高级工具。支持 `?page=library|agents|history|settings`，旧页面深链仍可打开对应入口。

### 首次使用、项目目录和软件设置

1. 空库页面直接选择「从已有 Agent 导入」或「手动添加」。无需先手写 YAML，首次保存时才创建文件。
   侧栏「当前配置库」可切换现有库或指定新路径，并可勾选设为启动默认库；损坏文件必须先修复或换库。
2. 顶部「全局配置」指当前用户跨项目的 Agent 配置，只应用到勾选的 Agent；此时隐藏项目目录。
3. 选择「项目配置」，点击项目名称或「选择项目目录」，选择系统目录或手动填写绝对路径。
   目录必须已存在；取消系统选择不保存设置，不会沿用程序启动目录。最近项目目录会持久化。
4. 在「我的 MCP」点击配置的「选择目标」，在右侧勾选需要应用的 Agent。每个 MCP 的目标独立保存。
   底栏出现「预览变更」，核对实际路径、共享文件提示和脱敏差异，再点击「确认写入」。保存配置库不会自动分发。
5. 「软件设置」保存默认配置库、备份目录、启动默认范围和写入策略。设置位于 `~/.mcp-hub/settings.json`。
   默认库、范围、策略在下次启动使用；保存不自动改变当前任务范围。新备份目录保存后生效，不迁移历史记录。
   设置自身备份保留在 `~/.mcp-hub/backups`。

参数逐项编辑，逗号、等号和空格原样保留；环境变量和请求头使用键值表，字面值默认遮掩。
已有的平台覆盖、标签、启用状态和超时设置在编辑时保留。对话框支持键盘焦点限制和 Escape；放弃未保存编辑需确认。

首次启动不会自动写入 Agent。默认配置库不存在时显示首次使用引导，也可直接进入 Agents 只读查看。
设置保存先备份原文，再原子写入并校验；损坏或不支持的设置会阻止操作，需在软件设置中核对并明确保存。
备份可能含有凭据，Windows 使用用户目录继承的权限，请妥善保管；不宣称自动加固 Windows ACL。
未保存编辑切页会提示。Agent 的只读查看仍不写入配置；可另行进入导入流程，将选中条目安全保存到配置库。

矩阵按单元格选择：例如只勾选「filesystem → Claude Code」和「github → Codex」，
预览与写入只处理这两个配对，不会向两个客户端同时分发两条配置。
空选择不写入；修改勾选、作用域、覆盖模式或重新加载清单后，需要重新生成预览。
未选中的已有 MCP 条目会保留，CLI 不指定条目选择时仍按原有方式处理清单。

从 v0.4.1 起提供分发一致性保护：确认必须使用后端签发的一次性预览 ID。配置库、目标文件、路径、设置、上下文或最终解析计划变化时，
整批停止写入并要求重新预览；新预览、取消、过期和确认尝试都会使旧令牌失效。拒绝原因在预览弹窗内显示，旧确认按钮会移除。
快照原文仅留在后端内存，预览有效期 10 分钟、单文件上限 4 MiB。CLI 即时命令保持原语义，旧 GUI 确认 IPC 不可绕过 ID 校验。
v0.4.0 及更早版本不含这些保护。详细范围和跨进程竞争限制见 [一致性说明](docs/SAFETY.md#分发预览与确认一致性)。

所有测试与构建放在 GitHub Actions，本地不运行。`CI` 验证 Windows/Linux Rust 测试、前端单元测试、
TypeScript 与 Vite 构建、Chrome 真实交互流程、Windows Tauri 编译及原生命令单元测试。
独立的 `GUI review` 工作流在 macOS Chrome 运行相同交互并保存带中文字体的截图，不新增依赖。
浏览器回归覆盖空库引导、损坏库保护、编辑值保真、未保存确认、逐对勾选、过期预览、项目目录取消与传递、
同名冲突和安全导入、部分失败、备份恢复、设置失败与重启恢复，以及小窗口布局。截图随工作流上传。
回归只使用内存 IPC 测试数据和临时目录，不读取或改写用户真实配置。原生功能与适配器由 Rust 测试补充验证。

### 查看 Agent 已有的 MCP

进入「Agents」，点选目标 Agent 卡片，展开条目即可查看已有配置的字段结构。
页面显示实际读取的配置路径、格式和 MCP 名称；未创建清单文件也能使用此功能。
配置文件不存在、存在但未配置 MCP、配置损坏会分别提示。读取沿用各客户端的有序候选路径，
多个候选存在时读取第一个；优先文件损坏时报告错误，不会自动换读其他文件。

查看入口只读，不会修改 Agent 配置、产生备份或启动 MCP 服务。点击「导入到 MCP 库」可进入独立的扫描、冲突处理和预览流程。
v0.4.2 起按字段和参数语义脱敏：普通 URL、包名、目录、参数和非敏感环境变量/请求头正常显示；
识别到的密钥、Token、密码、认证头及含凭据的地址在后端隐藏。已有变量引用保留。隐藏后的详情不用于直接复制配置。
项目级路径使用顶部明确选定的项目目录；未选择有效目录时阻止读取和写入，不支持的作用域不可读取。
切换 Agent、刷新或切换作用域后，旧请求不会覆盖当前查看结果。

相关回归由 GitHub CI 执行：后端在临时目录覆盖三平台的 14 个适配器、JSON/JSONC/TOML、
中文、候选路径和敏感值处理；真实浏览器覆盖查看、错误提示、缺少清单及异步响应顺序。

### 从已有 Agent 导入配置库

在 Agents 页点选 Agent 后点击「导入到 MCP 库」，勾选条目，确认凭据处理方式，再预览并确认。
同名项默认跳过，可重命名或明确替换；替换须额外确认，会先备份原配置库。未选中的条目和来源 Agent 文件保持不变。
普通参数、URL、目录及非敏感环境变量/请求头按原值导入。识别到的凭据默认原样复制到配置库（明文保存，请勿提交或分享配置库），也可逐个改为环境变量映射。
原始凭据不回传前端，扫描和预览中保持隐藏；无法无损转换的条目明确阻止导入。
确认时检查源文件、配置库和上下文是否变化，旧预览不能覆盖新内容。
具体变量规则、使用步骤、限制、GitHub 验证和回退方式见 [导入说明](docs/IMPORTING.md)。

## 命令行用法

```bash
mcp-hub clients                    # 列出 14 个客户端
mcp-hub doctor                     # 检测本机状态（四态 + 实际命中路径）
mcp-hub diff  --catalog customer-mcp.yaml --client claude-code,codex
mcp-hub apply --catalog customer-mcp.yaml --client claude-code,codex --yes
mcp-hub backups                    # 列出备份
mcp-hub restore --id 20261026-070423
```

安全默认：

- `apply` 不带 `--yes` **不写任何文件**，只列计划（退出码 2）
- `--dry-run` 可演练
- 每次写入前自动备份到 `~/.mcp-hub/backups/<id>/`
- `--home <dir>` 会把 HOME 与 APPDATA 一起隔离到该目录，适合演示/测试（不碰真实配置）
- `--mode merge|replace` 切换字段合并 / 整条替换
- `--scope user|project|local` 指定作用域（默认 user）

## 清单长什么样

见 [`examples/customer-mcp.yaml`](examples/customer-mcp.yaml)：

```yaml
version: 1
entries:
  - name: github
    transport: http
    url: https://api.githubcopilot.com/mcp/
    auth:
      type: bearer
      token_env: GITHUB_PAT      # 只写变量名，密钥不进清单
    scope: project               # 省略或 inherit = 跟随全局（默认 user）

  - name: stockdb
    transport: stdio
    command: python
    args: ["-u", "${PROJECT_ROOT}/ai_mcp/stockdb_full_mcp.py"]
    overrides:                   # 平台差异在 Resolver 合并，适配器不感知
      windows:
        command: c:/Dev/stockdata/.venv/Scripts/python.exe
```

## 发布（编译在 GitHub 上）

发布工作流先在 `dev` 预构建并验证产物，再由同一提交的 tag 创建 Release，无需本地编译。
修改发布工作流或 `docs/releases/` 下的版本说明并推送 `dev`，会触发预构建；
须等 `Release` 工作流成功且包含 `release-assets` 制品后，再在该提交打标签：

```bash
git tag v0.1.1
git push origin v0.1.1
```

GitHub Actions 会产出：

| 产物 | 平台 |
|---|---|
| `MCP.Hub_0.1.1_x64-setup.exe` | Windows 图形界面（未签名） |
| `MCP.Hub_0.1.1_aarch64.dmg` | macOS 图形界面（未签名，首次打开需右键「打开」） |
| `mcp-hub-v0.1.1-linux-x64.tar.gz` | Linux 命令行（musl 静态，免依赖） |
| `mcp-hub-v0.1.1-windows-x64.zip` / `-macos-arm64.tar.gz` | 命令行 |
| `SHA256SUMS` | 全部产物的校验和 |

> 从 v0.4.3 起产物改名以区分图形界面与命令行：macOS GUI 为 `MCP.Hub_<版本>_macos-arm64.dmg`（ad-hoc 签名），
> CLI 为 `mcp-hub-cli-v<版本>-<平台>.tar.gz` / `.zip`；Windows 安装程序与免安装 ZIP 名称不变。

另附 `customer-mcp.yaml` 示例清单。桌面安装后请在顶部清单输入框填写自有清单或下载示例的绝对路径。
发布前的测试、构建、安装包生成与校验均在 GitHub Actions 执行。
标签任务核对版本号和提交，复用相同提交的预构建产物，先上传至草稿 Release，
确认文件集合、大小及可用的服务端摘要匹配后才正式发布。缺少匹配预构建或制品已过期时会停止发布。
版本说明与已知限制见 [`docs/releases/v0.1.1.md`](docs/releases/v0.1.1.md)。

CI 在 `dev` 推送、提交到 `master` 的 PR 及发布预构建时运行：Windows/Linux 各 126 项 Rust 测试、
7 项前端测试、1 项真实浏览器回归测试、前端构建和 Windows 桌面编译检查。

## 构建前置条件

- 核心四层（core / io / app / cli）：rustc ≥ 1.85
- 桌面壳（mcp-desktop）：rustc ≥ 1.90（Tauri 2.10 的 MSRV），且不在 `default-members` 中
- crates.io 官方索引在国内网络下可能不可达：`cp .cargo/config.toml.example .cargo/config.toml` 走镜像（该文件已 gitignore）

## 设计约束（维护者须知）

1. **`mcp-core` 与未来的 `mcp-app` 不得依赖 `tauri` / `clap`**——业务逻辑与宿主解耦，换前端不动 Rust
2. **核心层无 IO**——M×N×K 的组合在纯函数里折叠，保证可穷举单测
3. **绝不整文件覆盖**——只改目标子树，字段级合并，保留未知字段
4. **密钥只存环境变量名**——明文会告警，内联需显式策略

## 开发

```bash
cargo fmt --all
cargo clippy -p mcp-core --all-targets -- -D warnings
cargo test -p mcp-core
```
