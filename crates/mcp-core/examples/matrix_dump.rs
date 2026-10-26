//! 把一个条目在 14 个客户端下的编码结果全部打印出来。
//!
//! 用途：直观核对「同一份清单 → 各家原生格式」的映射是否正确。
//! 运行：`cargo run -p mcp-core --example matrix_dump`

use mcp_core::adapters;
use mcp_core::interpolate::VarCtx;
use mcp_core::model::{Auth, McpEntry, Platform, Scope};
use mcp_core::resolve;
use std::collections::BTreeMap;

fn main() {
    let mut github = McpEntry::new_http("github", "https://api.githubcopilot.com/mcp/");
    github.auth = Some(Auth::env_ref("GITHUB_PAT"));

    let mut vars = BTreeMap::new();
    vars.insert("GITHUB_PAT".to_string(), "<从环境变量读取>".to_string());
    let ctx = VarCtx {
        home: Some("/home/dev".into()),
        project_root: Some("/work/proj".into()),
        vars,
        read_process_env: false,
    };

    println!("== 远程 + bearer：14 个客户端的落地形态 ==\n");
    for a in adapters::all() {
        let (resolved, _missing) = resolve::resolve(&github, Platform::Linux, &ctx);
        let value = a.encode_entry(&resolved).expect("encode");
        let scope = Scope::User;
        let fmt = format!("{:?}", a.spec().format);
        let rendered = serde_json::to_string_pretty(&value).unwrap();
        let root = a.root_key(scope).join(".");
        println!(
            "── {:<15} [{}] 根键 {}\n{}\n",
            a.display_name(),
            fmt,
            root,
            rendered
        );
    }

    println!("== OpenCode 禁用态（disabled 语义反转）==");
    let mut off = github.clone();
    off.enabled = false;
    let v = mcp_core::encode_entry("opencode", &off).unwrap();
    println!("{}\n", serde_json::to_string_pretty(&v).unwrap());

    println!("== 平台覆盖：stockdb 在 Windows / Linux 下的 command ==");
    let mut stock = McpEntry::new_stdio("stockdb", "python", vec![]);
    let ov_win = mcp_core::model::EntryOverride {
        command: Some("c:/Dev/stockdata/.venv/Scripts/python.exe".into()),
        ..Default::default()
    };
    stock.overrides.insert(Platform::Windows, ov_win);
    for p in [Platform::Windows, Platform::Linux] {
        let (r, _) = resolve::resolve(&stock, p, &ctx);
        println!("  {:<8} -> {}", p.as_str(), r.command.unwrap_or_default());
    }
}
