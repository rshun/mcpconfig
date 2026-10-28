use mcp_app::{App, HostEnv, PlanRequest};
use mcp_core::{adapters, catalog, Platform, Scope};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn fixture() -> (PathBuf, App, PlanRequest) {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "mcphub-safety-{}-{}-{serial}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(dir.join("project")).unwrap();
    let app = App::with_home(
        dir.join("data"),
        HostEnv {
            home: dir.clone(),
            appdata: Some(dir.join("AppData/Roaming")),
            project_root: Some(dir.join("project")),
        },
        Platform::Windows,
    );
    let path = dir.join("catalog.yaml");
    fs::write(&path, "version: 1\nentries:\n  - {name: alpha, transport: stdio, command: python}\n  - {name: beta, transport: stdio, command: node}\n").unwrap();
    (dir, app, PlanRequest::new(path))
}

#[test]
fn shared_file_is_written_once_with_all_selected_changes_and_one_backup() {
    let (dir, app, mut request) = fixture();
    let target = dir.join("project/.mcp.json");
    let original = r#"{"theme":"dark","mcpServers":{"keep":{"command":"keep"}}}"#;
    fs::write(&target, original).unwrap();
    request.scope = Scope::Project;
    request.selection = Some(BTreeMap::from([
        ("claude-code".into(), vec!["alpha".into()]),
        ("vscode".into(), vec!["beta".into()]),
    ]));
    let plan = app.plan(&request).unwrap();
    assert_eq!(plan.plans.len(), 1);
    assert_eq!(plan.writable(), 1);
    assert!(plan.plans[0].client_name.contains("VS Code"));
    let receipt = app.apply(&plan.plans, false).unwrap();
    assert_eq!(receipt.written(), 1);
    assert_eq!(receipt.failed(), 0);
    let actual: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(actual["mcpServers"]["alpha"]["command"], "python");
    assert_eq!(actual["mcpServers"]["beta"]["command"], "node");
    assert_eq!(actual["mcpServers"]["keep"]["command"], "keep");
    assert_eq!(actual["theme"], "dark");
    let backups = app.backups().unwrap();
    assert_eq!(backups[0].files.len(), 1);
    app.restore(receipt.backup_id.as_deref().unwrap()).unwrap();
    assert_eq!(fs::read_to_string(target).unwrap(), original);
}

#[test]
fn incompatible_shared_entry_is_blocked_without_writing() {
    let (dir, app, mut request) = fixture();
    let target = dir.join("project/.mcp.json");
    fs::write(&target, "{}").unwrap();
    request.scope = Scope::Project;
    request.selection = Some(BTreeMap::from([
        ("claude-code".into(), vec!["alpha".into()]),
        ("vscode".into(), vec!["alpha".into()]),
    ]));
    let plan = app.plan(&request).unwrap();
    assert_eq!(plan.writable(), 0);
    assert!(plan.plans[0]
        .blocked
        .as_deref()
        .unwrap()
        .contains("同名 MCP"));
    assert_eq!(app.apply(&plan.plans, false).unwrap().written(), 0);
    assert_eq!(fs::read_to_string(target).unwrap(), "{}");
}

#[test]
fn vscode_native_and_portable_paths_use_their_own_roots_for_plan_detect_and_inspect() {
    for (scope, relative, root) in [
        (Scope::User, "AppData/Roaming/Code/User/mcp.json", "servers"),
        (Scope::User, ".copilot/mcp-config.json", "mcpServers"),
        (Scope::Project, "project/.vscode/mcp.json", "servers"),
        (Scope::Project, "project/.mcp.json", "mcpServers"),
    ] {
        let (dir, app, mut request) = fixture();
        let target = dir.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            &target,
            json!({(root): {"keep": {"command":"keep"}}}).to_string(),
        )
        .unwrap();
        request.scope = scope;
        request.selection = Some(BTreeMap::from([("vscode".into(), vec!["alpha".into()])]));
        let plan = app.plan(&request).unwrap();
        assert_eq!(
            app.apply(&plan.plans, false).unwrap().written(),
            1,
            "{relative}"
        );
        let actual: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(actual[root]["alpha"]["command"], "python");
        assert_eq!(actual[root]["keep"]["command"], "keep");
        assert!(actual
            .get(if root == "servers" {
                "mcpServers"
            } else {
                "servers"
            })
            .is_none());
        let inspected = app.read_client_config("vscode", scope).unwrap();
        assert_eq!(inspected.entries.len(), 2);
        let detected =
            mcp_app::detect::detect_one("vscode", &app.env, app.platform, scope).unwrap();
        assert_eq!(detected.configured_entries.len(), 2);
    }
}

#[test]
fn commented_vscode_file_is_blocked_until_explicit_comment_strip_permission() {
    let (dir, app, mut request) = fixture();
    let target = dir.join("project/.vscode/mcp.json");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    let original = "{ // 注释\n\"servers\":{}}";
    fs::write(&target, original).unwrap();
    request.scope = Scope::Project;
    request.selection = Some(BTreeMap::from([("vscode".into(), vec!["alpha".into()])]));
    assert_eq!(app.plan(&request).unwrap().writable(), 0);
    assert_eq!(fs::read_to_string(&target).unwrap(), original);
    request.allow_comment_strip = true;
    let plan = app.plan(&request).unwrap();
    assert_eq!(app.apply(&plan.plans, false).unwrap().written(), 1);
}

#[test]
fn catalog_save_preserves_json_format_and_is_recoverable_through_normal_backups() {
    let (dir, app, _) = fixture();
    let path = dir.join("own-catalog.json");
    let old = r#"{"version":1,"entries":[]}"#;
    fs::write(&path, old).unwrap();
    let cat =
        catalog::parse("version: 1\nentries: [{name: local, transport: stdio, command: python}]\n")
            .unwrap();
    app.save_catalog(&path, &cat).unwrap();
    let saved = fs::read_to_string(&path).unwrap();
    assert!(serde_json::from_str::<Value>(&saved).is_ok());
    let backups = app.backups().unwrap();
    assert_eq!(backups[0].files[0].client_id, "catalog");
    app.restore(&backups[0].id).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), old);
}

#[test]
fn preview_never_exposes_existing_or_resolved_secret_values() {
    let (dir, app, mut request) = fixture();
    let target = app
        .target_path(adapters::by_id("codex").unwrap(), Scope::User)
        .unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "unrelated_password = 'your_unrelated_secret_here'\n[mcp_servers.alpha]\ncommand = 'python'\n[mcp_servers.alpha.env]\nAPI_KEY = 'your_old_key_here'\n").unwrap();
    fs::write(dir.join("catalog.yaml"), "version: 1\nentries: [{name: alpha, transport: stdio, command: python, env: {API_KEY: your_new_key_here}}]\n").unwrap();
    request.selection = Some(BTreeMap::from([("codex".into(), vec!["alpha".into()])]));
    let plan = app.plan(&request).unwrap();
    let shown = plan.plans[0].diff_text();
    let lines = format!("{:?}", plan.plans[0].line_diff);
    for value in [
        "your_unrelated_secret_here",
        "your_old_key_here",
        "your_new_key_here",
    ] {
        assert!(!shown.contains(value));
        assert!(!lines.contains(value));
    }
    assert!(shown.contains("原值") && shown.contains("新值"));
    assert_eq!(app.apply(&plan.plans, false).unwrap().written(), 1);
    assert!(
        fs::read_to_string(target)
            .unwrap()
            .contains("your_new_key_here"),
        "脱敏只能影响展示，不能改写实际配置"
    );
}
