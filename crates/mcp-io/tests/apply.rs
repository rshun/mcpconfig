//! 写入引擎集成测试：备份 / 原子写 / 校验回滚 / 恢复 / 演练。

use mcp_core::adapters::Format;
use mcp_core::document::{ChangeKind, MergeMode};
use mcp_core::plan::{plan_file, EntryChange, FilePlan};
use mcp_core::{adapters, McpEntry, Scope};
use mcp_io::{ApplyOptions, Engine, OutcomeStatus};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mcphub-it-{}-{}-{}", tag, std::process::id(), n));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    fn home(&self) -> PathBuf {
        self.root.join(".mcp-hub")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn github() -> McpEntry {
    let mut e = McpEntry::new_http("github", "https://api.githubcopilot.com/mcp/");
    e.auth = Some(mcp_core::Auth::env_ref("GITHUB_PAT"));
    e
}

#[test]
fn writes_file_creates_backup_and_audit() {
    let sb = Sandbox::new("basic");
    let target = sb.path("claude.json");
    let engine = Engine::new(sb.home());

    let a = adapters::by_id("claude-code").unwrap();
    let plan = plan_file(
        a,
        target.display().to_string(),
        Scope::User,
        None,
        &[github()],
        MergeMode::Merge,
        false,
    )
    .unwrap();
    assert!(plan.has_write());

    let receipt = engine.apply(&[plan], &ApplyOptions::default()).unwrap();
    assert_eq!(receipt.written(), 1);
    assert!(receipt.all_ok());
    assert!(receipt.backup_id.is_some());

    // 文件已写入且内容正确
    let text = fs::read_to_string(&target).unwrap();
    assert!(text.contains("github"));
    assert!(text.contains("Bearer ${GITHUB_PAT}"));

    // 备份目录与审计存在
    let bid = receipt.backup_id.unwrap();
    assert!(sb
        .home()
        .join("backups")
        .join(&bid)
        .join("manifest.json")
        .exists());
    let audit = fs::read_to_string(engine.audit_path()).unwrap();
    assert!(audit.contains("write"));
    assert!(audit.contains("claude-code"));
    assert!(!audit.contains("GITHUB_PAT"), "审计日志不得记录密钥");
}

#[test]
fn idempotent_second_apply_writes_nothing() {
    let sb = Sandbox::new("idem");
    let target = sb.path("claude.json");
    let engine = Engine::new(sb.home());
    let a = adapters::by_id("claude-code").unwrap();

    let p1 = plan_file(
        a,
        target.display().to_string(),
        Scope::User,
        None,
        &[github()],
        MergeMode::Merge,
        false,
    )
    .unwrap();
    engine.apply(&[p1], &ApplyOptions::default()).unwrap();

    let existing = fs::read_to_string(&target).unwrap();
    let p2 = plan_file(
        a,
        target.display().to_string(),
        Scope::User,
        Some(&existing),
        &[github()],
        MergeMode::Merge,
        false,
    )
    .unwrap();
    assert!(!p2.has_write(), "内容一致时不应产生写入");

    let receipt = engine.apply(&[p2], &ApplyOptions::default()).unwrap();
    assert_eq!(receipt.written(), 0);
    assert_eq!(receipt.outcomes[0].status, OutcomeStatus::Unchanged);
    assert!(receipt.backup_id.is_none(), "无写入时不应产生备份");
}

#[test]
fn keeps_user_entries_and_unrelated_keys() {
    let sb = Sandbox::new("preserve");
    let target = sb.path("claude.json");
    fs::write(
        &target,
        r#"{"mcpServers":{"mine":{"command":"node","args":["my.js"]}},"theme":"dark"}"#,
    )
    .unwrap();

    let engine = Engine::new(sb.home());
    let a = adapters::by_id("claude-code").unwrap();
    let existing = fs::read_to_string(&target).unwrap();
    let plan = plan_file(
        a,
        target.display().to_string(),
        Scope::User,
        Some(&existing),
        &[github()],
        MergeMode::Merge,
        false,
    )
    .unwrap();
    engine.apply(&[plan], &ApplyOptions::default()).unwrap();

    let text = fs::read_to_string(&target).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["theme"], "dark");
    assert_eq!(v["mcpServers"]["mine"]["command"], "node");
    assert!(v["mcpServers"]["github"].is_object());
}

#[test]
fn verify_failure_rolls_back() {
    let sb = Sandbox::new("rollback");
    let target = sb.path("bad.json");
    let engine = Engine::new(sb.home());

    // 人为构造一个会写出非法 JSON 的计划，模拟引擎缺陷
    let plan = FilePlan {
        client_id: "claude-code".into(),
        client_name: "Claude Code".into(),
        scope: Scope::User,
        path: target.display().to_string(),
        format: Format::Json,
        existed: false,
        blocked: None,
        changes: vec![EntryChange {
            name: "x".into(),
            kind: ChangeKind::New,
            before: None,
            after: serde_json::json!({}),
        }],
        old_content: String::new(),
        new_content: "{ 这不是合法 JSON ".into(),
        line_diff: Vec::new(),
        warnings: Vec::new(),
    };

    let receipt = engine.apply(&[plan], &ApplyOptions::default()).unwrap();
    assert_eq!(receipt.failed(), 1);
    assert!(!receipt.all_ok());
    assert!(!target.exists(), "校验失败应删除新建文件");
    assert!(receipt.outcomes[0]
        .message
        .as_ref()
        .unwrap()
        .contains("校验失败"));
}

#[test]
fn restore_recovers_previous_content() {
    let sb = Sandbox::new("restore");
    let target = sb.path("claude.json");
    let original = r#"{"mcpServers":{"old":{"command":"npx"}}}"#;
    fs::write(&target, original).unwrap();

    let engine = Engine::new(sb.home());
    let a = adapters::by_id("claude-code").unwrap();
    let plan = plan_file(
        a,
        target.display().to_string(),
        Scope::User,
        Some(original),
        &[github()],
        MergeMode::Merge,
        false,
    )
    .unwrap();
    let receipt = engine.apply(&[plan], &ApplyOptions::default()).unwrap();
    let bid = receipt.backup_id.clone().unwrap();

    assert_ne!(fs::read_to_string(&target).unwrap(), original);

    let restored = engine.restore(&bid).unwrap();
    assert_eq!(restored, 1);
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        original,
        "内容应完全恢复"
    );
}

#[test]
fn restore_removes_file_that_was_created() {
    let sb = Sandbox::new("restore-new");
    let target = sb.path("new.json");
    let engine = Engine::new(sb.home());

    let a = adapters::by_id("claude-code").unwrap();
    let plan = plan_file(
        a,
        target.display().to_string(),
        Scope::User,
        None,
        &[github()],
        MergeMode::Merge,
        false,
    )
    .unwrap();
    let receipt = engine.apply(&[plan], &ApplyOptions::default()).unwrap();
    let bid = receipt.backup_id.unwrap();
    assert!(target.exists());

    engine.restore(&bid).unwrap();
    assert!(!target.exists(), "回滚新建文件应删除它");
}

#[test]
fn dry_run_does_not_touch_disk() {
    let sb = Sandbox::new("dry");
    let target = sb.path("claude.json");
    let engine = Engine::new(sb.home());

    let a = adapters::by_id("claude-code").unwrap();
    let plan = plan_file(
        a,
        target.display().to_string(),
        Scope::User,
        None,
        &[github()],
        MergeMode::Merge,
        false,
    )
    .unwrap();

    let receipt = engine
        .apply(&[plan], &ApplyOptions { dry_run: true })
        .unwrap();
    assert!(!target.exists());
    assert!(receipt.backup_id.is_none());
    assert_eq!(receipt.outcomes[0].status, OutcomeStatus::DryRun);
}

#[test]
fn blocked_plan_is_skipped() {
    let sb = Sandbox::new("blocked");
    let target = sb.path("broken.json");
    fs::write(&target, "{ 坏掉的文件").unwrap();
    let engine = Engine::new(sb.home());

    let a = adapters::by_id("claude-code").unwrap();
    let existing = fs::read_to_string(&target).unwrap();
    let plan = plan_file(
        a,
        target.display().to_string(),
        Scope::User,
        Some(&existing),
        &[github()],
        MergeMode::Merge,
        false,
    )
    .unwrap();
    assert!(plan.blocked.is_some());

    let receipt = engine.apply(&[plan], &ApplyOptions::default()).unwrap();
    assert_eq!(receipt.blocked(), 1);
    assert_eq!(receipt.written(), 0);
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "{ 坏掉的文件",
        "原文件必须原样保留"
    );
    assert!(receipt.backup_id.is_none(), "被阻止的文件不应产生备份");
}

#[test]
fn list_backups_returns_latest_first() {
    let sb = Sandbox::new("list");
    let engine = Engine::new(sb.home());
    let a = adapters::by_id("claude-code").unwrap();

    for (i, name) in ["a.json", "b.json"].iter().enumerate() {
        let target = sb.path(name);
        let plan = plan_file(
            a,
            target.display().to_string(),
            Scope::User,
            None,
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        let r = engine.apply(&[plan], &ApplyOptions::default()).unwrap();
        let _ = r;
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let _ = i;
    }

    let backups = engine.list_backups().unwrap();
    assert_eq!(backups.len(), 2);
    assert!(backups[0].id >= backups[1].id, "应按时间倒序");
}

#[test]
fn toml_file_written_with_backup() {
    let sb = Sandbox::new("toml");
    let target = sb.path("config.toml");
    let original = "# 我的注释\n[features]\njs_repl = false\n";
    fs::write(&target, original).unwrap();

    let engine = Engine::new(sb.home());
    let codex = adapters::by_id("codex").unwrap();
    let plan = plan_file(
        codex,
        target.display().to_string(),
        Scope::User,
        Some(original),
        &[github()],
        MergeMode::Merge,
        false,
    )
    .unwrap();
    let receipt = engine.apply(&[plan], &ApplyOptions::default()).unwrap();
    assert_eq!(receipt.written(), 1);

    let text = fs::read_to_string(&target).unwrap();
    assert!(text.contains("# 我的注释"), "注释必须保留");
    assert!(text.contains("[features]"), "无关表必须保留");
    assert!(
        text.contains("bearer_token_env_var"),
        "Codex 应使用原生字段"
    );
}
