# MCP Hub

多客户端 / 多环境 MCP 配置一键分发工具。

把一份客户自维护的清单，按各客户端的原生格式写入 14 个 MCP 客户端——不用手工配 M×N×K 次。

## 现在的状态

| 模块 | 状态 |
|---|---|
| `mcp-core`：IR、清单解析、插值、平台覆盖、14 个适配器 | ✅ 实现，67 项测试 |
| `mcp-io`：备份 / 原子写 / 写后校验 / 回滚 / 审计 | ✅ 实现，13 项测试 |
| `mcp-app` 应用服务层（检测 / 计划 / 应用 / 备份） | ✅ 实现，16 项测试 |
| `mcp-hub` CLI（doctor / diff / apply / backups / restore） | ✅ 实现，可执行 |
| GUI 前端（React + TS，5 屏） | ✅ 实现，可在浏览器评审（mock 数据） |
| GUI 桌面壳（Tauri v2 薄适配） | ✅ 代码完成；编译需 rustc ≥ 1.90，交给 CI 验证 |

设计文档见 [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)，客户端方言细则见 [`docs/adapters.md`](docs/adapters.md)，
可交互原型见 [`docs/prototype/index.html`](docs/prototype/index.html)。

## 支持的客户端（14）

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

界面包含 5 屏：客户端配置（矩阵）· MCP配置（编辑）· 差异预览 · 环境检测 · 备份回滚。
支持深链：`?page=matrix|catalog|diff|detect|backups`。

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

打 tag 即发布，无需本地编译：

```bash
git tag v0.1.0
git push origin v0.1.0
```

GitHub Actions 会产出：

| 产物 | 平台 |
|---|---|
| `MCP Hub_0.1.0_x64-setup.exe` | Windows 图形界面（未签名） |
| `MCP Hub_0.1.0_aarch64.dmg` | macOS 图形界面（未签名，首次打开需右键「打开」） |
| `mcp-hub-v0.1.0-linux-x64.tar.gz` | Linux 命令行（musl 静态，免依赖） |
| `mcp-hub-v0.1.0-windows-x64.zip` / `-macos-arm64.tar.gz` | 命令行 |
| `SHA256SUMS` | 全部产物的校验和 |

CI 每次 PR 还会跑：格式、clippy、96 项测试、前端构建、桌面壳编译检查，
以及一条**架构护栏**（断言 core/app 不依赖 tauri/clap）。

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
