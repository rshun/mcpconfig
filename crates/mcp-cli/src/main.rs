//! MCP Hub CLI：Linux / CI / 服务器上的唯一形态。

use clap::{Args, Parser, Subcommand};
use mcp_app::{App, HostEnv, PlanRequest};
use mcp_core::model::Scope;
use mcp_core::plan::MergeMode;
use mcp_io::OutcomeStatus;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "mcp-hub",
    version,
    about = "把一份客户清单分发到 14 个 MCP 客户端",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 列出支持的客户端
    Clients,
    /// 检测本机客户端状态（三通道 + 四态）
    Doctor {
        #[arg(long, default_value = "user")]
        scope: String,
        /// 只看某个客户端
        #[arg(long)]
        client: Option<String>,
        /// 自定义 HOME（测试用）
        #[arg(long)]
        home: Option<PathBuf>,
        /// 项目根目录（用于 <proj> 与 ${PROJECT_ROOT}）
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// 预演：展示将要发生的变化，不写盘
    Diff(PlanArgs),
    /// 应用：写入配置（默认需要 --yes）
    Apply {
        #[command(flatten)]
        plan: PlanArgs,
        /// 确认执行
        #[arg(long)]
        yes: bool,
        /// 只演练，不落盘
        #[arg(long)]
        dry_run: bool,
    },
    /// 列出备份
    Backups {
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// 回滚到指定备份
    Restore {
        #[arg(long)]
        id: String,
        #[arg(long)]
        home: Option<PathBuf>,
    },
}

#[derive(Args, Debug, Clone)]
struct PlanArgs {
    /// 客户清单文件（YAML / JSON）
    #[arg(long)]
    catalog: PathBuf,
    /// 目标客户端，逗号分隔；省略表示全部
    #[arg(long)]
    client: Option<String>,
    /// 作用域：user / project / local
    #[arg(long, default_value = "user")]
    scope: String,
    /// 覆盖方式：merge（字段合并）/ replace（整条替换）
    #[arg(long, default_value = "merge")]
    mode: String,
    /// 项目根目录
    #[arg(long)]
    project: Option<PathBuf>,
    /// JSONC 文件含注释时允许重写（会丢注释）
    #[arg(long)]
    allow_comment_strip: bool,
    /// 自定义 HOME（测试用）
    #[arg(long)]
    home: Option<PathBuf>,
}

/// 写 stdout 并忽略断管错误（管道被下游关闭时不应 panic）。
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("错误：{msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, String> {
    match cli.cmd {
        Cmd::Clients => {
            out!(
                "{} {} {} {} 支持作用域",
                pad("#", 4),
                pad("id", 16),
                pad("名称", 18),
                pad("类型", 6)
            );
            for (i, a) in mcp_core::adapters::all().iter().enumerate() {
                let scopes: Vec<&str> = a.spec().scopes.iter().map(|s| s.as_str()).collect();
                out!(
                    "{} {} {} {} {}",
                    pad(&(i + 1).to_string(), 4),
                    pad(a.id(), 16),
                    pad(a.display_name(), 18),
                    pad(&a.kind().to_string(), 6),
                    scopes.join(",")
                );
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Doctor {
            scope,
            client,
            home,
            project,
        } => {
            let scope = parse_scope(&scope)?;
            let app = make_app(home, project);
            let statuses = match client {
                Some(id) => vec![
                    mcp_app::detect::detect_one(&id, &app.env, app.platform, scope)
                        .ok_or_else(|| format!("未知客户端：{id}"))?,
                ],
                None => mcp_app::detect_all(&app.env, app.platform, scope),
            };

            out!("平台 {} · 作用域 {}", app.platform, scope);
            out!(
                "{} {} {} {} {}",
                pad("客户端", 18),
                pad("类型", 6),
                pad("状态", 16),
                pad("已配置", 8),
                "配置路径"
            );
            for s in &statuses {
                let path = s
                    .chosen_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "-".into());
                let state = match &s.scope_block {
                    Some(reason) => format!("不支持（{}）", reason),
                    None => s.state.label().to_string(),
                };
                out!(
                    "{} {} {} {} {}",
                    pad(&s.name, 18),
                    pad(&s.kind.to_string(), 6),
                    pad(&state, 16),
                    pad(&s.configured_entries.len().to_string(), 8),
                    path
                );
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Diff(args) => {
            let app = make_app(args.home.clone(), args.project.clone());
            let req = to_request(&args)?;
            let out = app.plan(&req).map_err(|e| e.to_string())?;
            print_plan_header(&out);

            for p in &out.plans {
                let (new, upd) = p.changed_count();
                if let Some(reason) = &p.blocked {
                    out!("✗ {:<15} 跳过：{reason}", p.client_name);
                    continue;
                }
                if !p.has_write() {
                    out!("· {:<15} 无变化  {}", p.client_name, p.path);
                    continue;
                }
                out!(
                    "→ {:<15} 新增 {new} / 修改 {upd}  {}",
                    p.client_name,
                    p.path
                );
                for w in &p.warnings {
                    out!("  ! {w}");
                }
                let d = p.diff_text();
                for line in d.lines() {
                    out!("    {line}");
                }
                out!();
            }

            let n = out.writable();
            if n == 0 {
                out!("没有需要写入的文件。");
            } else {
                out!("共 {n} 个文件待写入。执行：加 --yes 或改用 apply 子命令。");
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Apply { plan, yes, dry_run } => {
            let app = make_app(plan.home.clone(), plan.project.clone());
            let req = to_request(&plan)?;
            let out = app.plan(&req).map_err(|e| e.to_string())?;
            print_plan_header(&out);

            if !yes && !dry_run {
                for p in &out.plans {
                    if p.has_write() {
                        let (new, upd) = p.changed_count();
                        out!(
                            "→ {:<15} 新增 {new} / 修改 {upd}  {}",
                            p.client_name,
                            p.path
                        );
                    } else if let Some(r) = &p.blocked {
                        out!("✗ {:<15} 跳过：{r}", p.client_name);
                    }
                }
                out!("\n未写入任何文件。确认无误后加 --yes 执行。");
                return Ok(ExitCode::from(2));
            }

            let receipt = app.apply(&out.plans, dry_run).map_err(|e| e.to_string())?;

            out!(
                "{} {} 个文件 · 跳过 {} · 失败 {}",
                if dry_run { "演练" } else { "写入" },
                receipt.written(),
                receipt.blocked(),
                receipt.failed()
            );
            if let Some(bid) = &receipt.backup_id {
                out!("备份：{}", app.engine.backups_dir().join(bid).display());
            }
            for o in &receipt.outcomes {
                let msg = o.message.clone().unwrap_or_default();
                if o.status == OutcomeStatus::Unchanged {
                    continue;
                }
                out!("  [{}] {} {}", o.status.label(), o.client_id, msg);
            }

            Ok(if receipt.failed() > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }

        Cmd::Backups { home } => {
            let app = make_app(home, None);
            let list = app.backups().map_err(|e| e.to_string())?;
            if list.is_empty() {
                out!("暂无备份（{}）", app.engine.backups_dir().display());
                return Ok(ExitCode::SUCCESS);
            }
            out!("{:<20} {:<22} {}", "备份 ID", "时间", "文件数");
            for b in list {
                out!("{:<20} {:<22} {}", b.id, b.created_at, b.files.len());
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Restore { id, home } => {
            let app = make_app(home, None);
            let n = app.restore(&id).map_err(|e| e.to_string())?;
            out!("已回滚 {id}，恢复 {n} 个文件。");
            Ok(ExitCode::SUCCESS)
        }
    }
}

// ---------------------------------------------------------------- 工具

fn make_app(home: Option<PathBuf>, project: Option<PathBuf>) -> App {
    match home {
        Some(h) => {
            let mut env = HostEnv::from_process(project);
            // 关键：连同 %APPDATA% 一起隔离，避免 --home 仍写入真实用户目录
            env.appdata = Some(h.join("AppData/Roaming"));
            env.home = h.clone();
            App::with_home(
                h.join(".mcp-hub"),
                env,
                mcp_core::model::Platform::current(),
            )
        }
        None => App::from_process(project),
    }
}

fn to_request(args: &PlanArgs) -> Result<PlanRequest, String> {
    let mut req = PlanRequest::new(args.catalog.clone());
    req.scope = parse_scope(&args.scope)?;
    req.mode = parse_mode(&args.mode)?;
    req.allow_comment_strip = args.allow_comment_strip;
    req.clients = args.client.as_ref().map(|s| {
        s.split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect()
    });
    Ok(req)
}

fn parse_scope(s: &str) -> Result<Scope, String> {
    match s {
        "user" | "global" => Ok(Scope::User),
        "project" => Ok(Scope::Project),
        "local" => Ok(Scope::Local),
        other => Err(format!("未知作用域 {other}（可选 user / project / local）")),
    }
}

fn parse_mode(s: &str) -> Result<MergeMode, String> {
    match s {
        "merge" => Ok(MergeMode::Merge),
        "replace" => Ok(MergeMode::Replace),
        other => Err(format!("未知覆盖方式 {other}（可选 merge / replace）")),
    }
}

fn print_plan_header(out: &mcp_app::PlanOutcome) {
    out!("清单条目 {} 条（本作用域）", out.entry_count);
    for i in &out.catalog_issues {
        out!("  ! {i}");
    }
    for v in &out.missing_vars {
        out!("  ! 未解析变量 {v}（环境变量未设置）");
    }
    for (id, reason) in &out.skipped {
        out!("  · 跳过 {id}：{reason}");
    }
    if !out.skipped.is_empty() || !out.catalog_issues.is_empty() || !out.missing_vars.is_empty() {
        out!();
    }
}

/// 显示宽度：CJK 字符按 2 列计算。
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let u = c as u32;
            let wide = matches!(u,
                0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
                | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6
                | 0x20000..=0x3FFFD);
            if wide {
                2
            } else {
                1
            }
        })
        .sum()
}

fn pad(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}
