# MCP Hub 交互原型

单文件、无依赖、可离线打开。直接用浏览器打开 `index.html`。

支持深链，方便逐屏查看：

- `index.html#matrix` 客户端配置（默认）
- `index.html#catalog` MCP配置
- `index.html#diff` 差异预览
- `index.html#detect` 环境检测
- `index.html#backup` 备份回滚
- `index.html#cli` 命令行（Linux）

---

## 建议的点击路径（5 分钟走完核心闭环）

1. **客户端配置页**：14 个客户端全上（表格超宽，可横向滚动，首列冻结），默认已勾选 3 个 MCP × 6 个客户端。
   - 点任意格子可切换选中，底部计数实时变化
   - 颜色：绿点=已配置一致，空心=未配置，琥珀点=有差异（`github` 在 Codex 列）
   - 行首标签是**该条配置的生效作用域**；`—` 表示该 Agent 不支持此作用域
   - 悬停格子可看到「作用域 → 写入路径」
2. 点 **一键应用** → 进入 **差异预览**。
   - 重点看 `github` 在不同 Agent 下的 bearer 写法差异：
     - Claude Code → `"Bearer ${GITHUB_PAT}"`
     - Codex → `bearer_token_env_var = "GITHUB_PAT"`（TOML）
     - OpenCode → `"Bearer {env:GITHUB_PAT}"`（语法不同）
     - Antigravity → 触发 `<需内联:GITHUB_PAT>` 告警 + 琥珀提示条
   - 注意同一 Agent 的不同作用域被拆成不同文件分组（`github` 是项目级，落在 `<proj>\.mcp.json`）
   - 顶部可切换 `secret_policy`；点 **确认写入** 出现完成回执
3. **MCP配置**页：左侧切条目，右侧表单可编辑。
   - 改「作用域」下拉，客户端配置页对应标签立即变化
   - 改「地址 / Bearer 变量」，右侧 YAML 实时更新
   - 可**新增配置**、**删除此条**
4. **环境检测**页：四态、支持作用域标签、当前作用域路径、路径覆盖入口
5. **备份回滚**页：备份记录 + 审计日志（不含密钥值）
6. **命令行页**：Linux 只有 CLI 时的实际形态

---

## 作用域（scope）模型

**作用域不是条目自身的属性，而是「条目 × Agent」的属性** —— 因为各 Agent 支持的作用域不同：

| 客户端 | 用户级 | 项目级 | 本地级 |
|---|---|---|---|
| Claude Code | ✓ `~/.claude.json` | ✓ `<proj>/.mcp.json` | ✓ `.claude.json` 内按项目嵌套 |
| Codex | ✓ `~/.codex/config.toml` | ✓ `<proj>/.codex/config.toml` | — |
| OpenCode | ✓ `~/.config/opencode/opencode.json` | ✓ `<proj>/opencode.json` | — |
| VS Code | ✓ `%APPDATA%/Code/User/mcp.json`（根键 `servers`） | ✓ `<proj>/.vscode/mcp.json` | — |
| Antigravity | ✓ `~/.gemini/config/mcp_config.json` | ✓ `<proj>/.agents/mcp_config.json`（workspace） | — |
| Grok | ✓ `~/.grok/config.toml` | ✓ `<proj>/.grok/config.toml` | — |
| Kimi Code | ✓ | — | — |
| WorkBuddy | ✓ | — | — |

解析规则：

```
生效作用域 = 条目.scope == 'inherit' ? 顶部全局作用域 : 条目.scope
```

- 顶部「作用域」下拉 = 全局默认（新条目默认继承）
- 条目可单独覆盖（客户端配置页名称旁的标签会标「条目指定」或「·继承」）
- 目标 Agent 不支持该作用域 → 格子禁用，差异页列为「已跳过」

> **本地级（local）** 仅 Claude Code 支持：只对本机本项目生效，不随仓库分发给他人。

---

## 客户端配置左列从哪来

「客户端配置」页左列的 `stockdb` / `github` 等，**就是清单文件 `customer-mcp.yaml` 的 `entries`**：

- 顶部「目录」下拉选择加载哪个清单
- 「MCP配置」页增删改，保存后写回该 YAML
- 编辑任一条目，客户端配置页立即变化（同一份数据）

三种录入途径：① 直接编辑 YAML 文件（可入库、可共享）；② MCP配置页表单增删改；
③ 从现有 Agent 配置反向导入（原型未展开）。

> 原型里的初始条目写死在 `index.html` 的 `ENTRIES` 数组，仅为演示；产品中由 YAML 文件驱动。

---

## 原型里固化的设计决策

| 决策 | 在原型中的体现 |
|---|---|
| 客户端配置是主界面 | 一屏覆盖 M×N，勾选即目标 |
| 三态可视化 | 绿 / 空心 / 琥珀，另有「有差异」独立色 |
| 作用域是一等维度 | 全局默认 + 条目覆盖；不支持的格子禁用；同一 Agent 不同作用域拆成不同文件 |
| 未勾选 ≠ 删除 | 图例明确写出，删除需在差异页显式勾选 |
| bearer 三档映射 | 同一 `github` 条目在 4 个 Agent 下写出 4 种形态 |
| 密钥风险可控 | `secret_policy` 可切换，内联处有告警条，回执说明备份权限 0700 |
| 平台覆盖在 Resolver 合并 | 切平台只改「实际命令」，适配器不感知 |
| Linux 仅 CLI | 切到 Linux 平台，客户端配置替换为 CLI 用法卡片 |
| 侦测与写入解耦 | 环境检测只显示状态与路径，不会擅自合并现有配置 |

---

## 已确认的产品决策（2026-10-26）

| # | 决策 |
|---|---|
| D1 | **14 个客户端全上**：Claude Code、Codex、WorkBuddy、Kimi Code、OpenCode(v2)、VS Code、Antigravity、Grok、Cursor、Cline、Gemini CLI、Claude Desktop、Windsurf、Trae |
| D2 | **OpenCode 只支持 v2**：不做 v1 兼容；检测到 v1 结构只提示迁移 |
| D3 | **只导出配置**：Local + Bundle，不做 SSH 推送 |
| D4 | **只做客户自有清单**：不接公共 Registry，工具离线可用 |
| D5 | **默认用户级（全局）**：条目可逐条覆盖 |
| D6 | **GUI 只要漂亮**：Tauri v2 + React + shadcn/ui + Tailwind（见 ARCHITECTURE §8.4） |
| D7 | **macOS 不签名** |
| D8 | **不做自动更新** |

---

## 已知简化（原型阶段）

- 差异预览里的 `${PROJECT_ROOT}`、`${ENV:...}` 未插值；实际写入前由 Resolver 解析
- 差异页的「删除条目」勾选交互未做
- Bundle 导出/导入的目标选择界面未做
- 自定义 Agent 的声明表单（路径 / 根键 / 命令写法 / 变量键）未做
- 平台覆盖仅展示，无编辑入口
- 反向导入（扫描现有客户端配置）未做
- 14 列的列分组（CLI/IDE/Desktop）与列隐藏未做，当前只有横向滚动
