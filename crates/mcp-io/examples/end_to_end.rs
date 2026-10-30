//! 端到端演示：清单 → 计划 → diff → 写入 → 回滚。
//!
//! 全部在临时目录内运行，**不会碰真实配置**。
//! 运行：`cargo run -p mcp-io --example end_to_end`

use mcp_core::catalog;
use mcp_core::interpolate::VarCtx;
use mcp_core::model::{Platform, Scope};
use mcp_core::plan::{plan_file, MergeMode};
use mcp_core::resolve;
use mcp_core::{adapters, McpEntry};
use mcp_io::{ApplyOptions, Engine};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let catalog_path = manifest.join("../../examples/customer-mcp.yaml");
    let src = fs::read_to_string(&catalog_path)?;

    let (cat, issues) = catalog::load(&src)?;
    println!("清单：{} 条，{} 个提示", cat.entries.len(), issues.len());
    for i in &issues {
        println!("  {i}");
    }

    // 模拟环境：不读真实进程环境，避免演示受本机影响
    let mut vars = BTreeMap::new();
    vars.insert(
        "STOCKDB_URL".to_string(),
        "postgres://localhost/stock".to_string(),
    );
    vars.insert("GITHUB_PAT".to_string(), "ghp_demo".to_string());
    vars.insert("CONTEXT7_API_KEY".to_string(), "ctx7_demo".to_string());
    let ctx = VarCtx {
        home: Some("C:/Users/demo".into()),
        project_root: Some("C:/Dev/stockdata".into()),
        vars,
        read_process_env: false,
    };

    let platform = Platform::current();
    let (resolved, missing) = resolve::resolve_catalog(&cat, platform, &ctx);
    println!(
        "\n平台 {}：解析后 {} 条，未解析变量 {} 个",
        platform.as_str(),
        resolved.len(),
        missing.len()
    );
    for m in &missing {
        println!("  ! 未设置：{}", m.raw);
    }

    // 临时工作区
    let work = std::env::temp_dir().join(format!("mcphub-demo-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work)?;
    let engine = Engine::new(work.join(".mcp-hub"));

    // 预置一份「用户已有配置」，演示保留行为
    let claude_demo = work.join("claude.json");
    fs::write(
        &claude_demo,
        r#"{"theme":"dark","mcpServers":{"my-private":{"command":"node","args":["p.js"]}}}"#,
    )?;

    let targets = [
        ("claude-code", claude_demo.clone()),
        ("codex", work.join("config.toml")),
        ("opencode", work.join("opencode.json")),
    ];

    let mut plans = Vec::new();
    for (client, path) in &targets {
        let adapter = adapters::by_id(client).ok_or_else(|| format!("未知客户端 {client}"))?;
        let scope = Scope::User;
        let entries: Vec<McpEntry> = resolve::entries_for_scope(&resolved, Scope::User, scope);
        let existing = engine.read_text(path)?;
        let plan = plan_file(
            adapter,
            path.display().to_string(),
            scope,
            existing.as_deref(),
            &entries,
            MergeMode::Merge,
            false,
        )?;
        plans.push(plan);
    }

    println!("\n== 计划 ==");
    for p in &plans {
        let (new, upd) = p.changed_count();
        if let Some(reason) = &p.blocked {
            println!("  ✗ {:<12} 阻止：{reason}", p.client_id);
        } else if p.has_write() {
            println!(
                "  ✓ {:<12} 新增 {new} / 修改 {upd} → {}",
                p.client_id, p.path
            );
        } else {
            println!("  · {:<12} 无变化", p.client_id);
        }
    }

    if let Some(first) = plans.iter().find(|p| p.has_write()) {
        println!("\n== {} 的 diff（前 20 行）==", first.client_name);
        for line in first.diff_text().lines().take(20) {
            println!("{line}");
        }
    }

    let receipt = engine.apply(&plans, &ApplyOptions::default())?;
    println!("\n== 写入回执 ==");
    println!(
        "写入 {} 个文件 · 阻止 {} · 失败 {} · 备份 {}",
        receipt.written(),
        receipt.blocked(),
        receipt.failed(),
        receipt.backup_id.as_deref().unwrap_or("-")
    );
    for o in &receipt.outcomes {
        let msg = o.message.clone().unwrap_or_default();
        println!("  [{}] {} {}", o.status.label(), o.client_id, msg);
    }

    println!("\n== 写入后的 claude.json（注意 my-private 与 theme 都被保留）==");
    println!("{}", fs::read_to_string(&claude_demo)?);

    println!("== codex config.toml（TOML 方言）==");
    println!("{}", fs::read_to_string(work.join("config.toml"))?);

    // 回滚演示
    if let Some(bid) = &receipt.backup_id {
        let n = engine.restore(bid)?;
        println!("== 回滚 {bid}：恢复 {n} 个文件 ==");
        println!("{}", fs::read_to_string(&claude_demo)?);
    }

    let _ = fs::remove_dir_all(&work);
    Ok(())
}
