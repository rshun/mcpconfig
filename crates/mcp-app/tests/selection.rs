use mcp_app::{App, AppError, HostEnv, PlanRequest};
use mcp_core::adapters;
use mcp_core::document::ConfigDoc;
use mcp_core::{plan::MergeMode, Platform, Scope};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn fixture(tag: &str) -> (std::path::PathBuf, App, PlanRequest) {
    let dir = std::env::temp_dir().join(format!(
        "mcphub-selection-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let catalog_path = dir.join("catalog.yaml");
    fs::write(
        &catalog_path,
        r#"version: 1
entries:
  - { name: alpha, transport: stdio, command: alpha-new }
  - { name: beta, transport: stdio, command: beta-new }
  - { name: project-only, transport: stdio, command: scoped, scope: project }
"#,
    )
    .unwrap();
    let host = HostEnv {
        home: dir.clone(),
        appdata: Some(dir.join("AppData/Roaming")),
        project_root: Some(dir.join("project")),
    };
    let app = App::with_home(dir.join(".mcp-hub"), host, Platform::Windows);
    (dir, app, PlanRequest::new(catalog_path))
}

fn selection(cells: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    cells
        .iter()
        .map(|(id, names)| {
            (
                id.to_string(),
                names.iter().map(|name| name.to_string()).collect(),
            )
        })
        .collect()
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn entry(app: &App, id: &str, name: &str) -> Option<Value> {
    let adapter = adapters::by_id(id).unwrap();
    let path = app.target_path(adapter, Scope::User).unwrap();
    let text = fs::read_to_string(path).unwrap();
    ConfigDoc::parse(adapter.spec().format, &text)
        .unwrap()
        .get(&adapter.root_key(Scope::User), name)
}

#[test]
fn paired_selection_preserves_unselected_entries_and_clients_in_both_modes() {
    for mode in [MergeMode::Merge, MergeMode::Replace] {
        let (dir, app, mut req) = fixture("paired");
        write(
            &dir.join(".claude.json"),
            r#"{"preferences":{"theme":"dark"},"mcpServers":{
                "alpha":{"command":"alpha-old","custom":"keep"},
                "beta":{"command":"beta-untouched","custom":"keep"}}}"#,
        );
        write(
            &dir.join(".codex/config.toml"),
            "model = \"unchanged\"\n[mcp_servers.alpha]\ncommand = \"alpha-untouched\"\ncustom = 42\n[mcp_servers.beta]\ncommand = \"beta-old\"\n",
        );
        let other_path = dir.join(".cursor/mcp.json");
        let other_content = "{\"mcpServers\":{\"alpha\":{\"command\":\"cursor-untouched\"}}}\n";
        write(&other_path, other_content);
        let claude_beta = entry(&app, "claude-code", "beta");
        let codex_alpha = entry(&app, "codex", "alpha");
        req.selection = Some(selection(&[
            ("claude-code", &["alpha"]),
            ("codex", &["beta"]),
        ]));
        req.mode = mode;

        let out = app.plan(&req).unwrap();
        assert_eq!(out.entry_count, 2);
        assert_eq!(out.plans.len(), 2);
        for plan in &out.plans {
            let expected = if plan.client_id == "claude-code" {
                "alpha"
            } else {
                "beta"
            };
            assert_eq!(plan.changes.len(), 1);
            assert_eq!(plan.changes[0].name, expected);
        }
        let receipt = app.apply(&out.plans, false).unwrap();
        assert_eq!(receipt.failed(), 0);
        assert_eq!(receipt.written(), 2);
        assert_eq!(entry(&app, "claude-code", "beta"), claude_beta);
        assert_eq!(entry(&app, "codex", "alpha"), codex_alpha);
        assert_eq!(entry(&app, "claude-code", "alpha").unwrap()["command"], "alpha-new");
        assert_eq!(entry(&app, "codex", "beta").unwrap()["command"], "beta-new");
        let claude: Value = serde_json::from_str(&fs::read_to_string(dir.join(".claude.json")).unwrap()).unwrap();
        assert_eq!(claude["preferences"], json!({"theme": "dark"}));
        assert!(fs::read_to_string(dir.join(".codex/config.toml")).unwrap().contains("model = \"unchanged\""));
        assert_eq!(fs::read_to_string(other_path).unwrap(), other_content);
        assert_eq!(app.plan(&req).unwrap().writable(), 0, "相同选择重复应用应幂等");
    }
}

#[test]
fn empty_selection_never_falls_back_to_all_entries_or_clients() {
    for cells in [selection(&[]), selection(&[("codex", &[])])] {
        let (dir, app, mut req) = fixture("empty");
        let original = "{\"mcpServers\":{\"alpha\":{\"command\":\"keep\"}}}";
        write(&dir.join(".claude.json"), original);
        req.selection = Some(cells);
        let out = app.plan(&req).unwrap();
        assert_eq!(out.entry_count, 0);
        assert!(out.plans.is_empty());
        let receipt = app.apply(&out.plans, false).unwrap();
        assert_eq!(receipt.written(), 0);
        assert!(receipt.backup_id.is_none());
        assert!(!dir.join(".codex/config.toml").exists());
        assert!(!app.engine.backups_dir().exists());
        assert_eq!(fs::read_to_string(dir.join(".claude.json")).unwrap(), original);
    }
}

#[test]
fn multiple_selected_entries_are_deduplicated_within_one_client() {
    let (_, app, mut req) = fixture("multiple");
    req.selection = Some(selection(&[("codex", &["alpha", "beta", "alpha"])]));
    let out = app.plan(&req).unwrap();
    assert_eq!(out.entry_count, 2);
    assert_eq!(out.plans.len(), 1);
    assert_eq!(out.plans[0].changes.len(), 2);
    app.apply(&out.plans, false).unwrap();
    assert_eq!(entry(&app, "codex", "alpha").unwrap()["command"], "alpha-new");
    assert_eq!(entry(&app, "codex", "beta").unwrap()["command"], "beta-new");
}

#[test]
fn unknown_and_out_of_scope_entries_fail_before_any_write() {
    for name in ["missing", "project-only"] {
        let (dir, app, mut req) = fixture("invalid");
        req.selection = Some(selection(&[
            ("claude-code", &["alpha"]),
            ("codex", &[name]),
        ]));
        assert!(matches!(app.plan(&req), Err(AppError::InvalidSelection(_))));
        assert!(!dir.join(".claude.json").exists());
        assert!(!dir.join(".codex/config.toml").exists());
        assert!(!app.engine.backups_dir().exists());
    }
}

#[test]
fn unknown_clients_in_selection_fail_closed() {
    let (_, app, mut req) = fixture("unknown-client");
    req.selection = Some(selection(&[("unknown", &["alpha"])]));
    assert!(matches!(app.plan(&req), Err(AppError::UnknownClient(_))));
}

#[test]
fn client_filter_cannot_expand_or_silently_ignore_a_selection() {
    let (_, app, mut req) = fixture("client-filter");
    req.clients = Some(vec!["claude-code".into(), "codex".into()]);
    req.selection = Some(selection(&[("claude-code", &["alpha"])]));
    let out = app.plan(&req).unwrap();
    assert_eq!(out.plans.len(), 1);
    assert_eq!(out.plans[0].client_id, "claude-code");
    req.clients = Some(vec!["codex".into()]);
    assert!(matches!(app.plan(&req), Err(AppError::InvalidSelection(_))));
}

#[test]
fn unselected_entries_do_not_contribute_missing_variable_warnings() {
    let (_, app, mut req) = fixture("variables");
    fs::write(
        &req.catalog_path,
        "version: 1\nentries:\n  - { name: alpha, transport: stdio, command: selected }\n  - name: beta\n    transport: stdio\n    command: other\n    args: ['${ENV:MCP_HUB_UNSELECTED_TEST_MISSING_946FD87A}']\n",
    )
    .unwrap();
    req.selection = Some(selection(&[("codex", &["alpha"])]));
    assert!(app.plan(&req).unwrap().missing_vars.is_empty());
    req.selection = Some(selection(&[("codex", &["beta"])]));
    assert_eq!(app.plan(&req).unwrap().missing_vars.len(), 1);
}

#[test]
fn absent_selection_preserves_the_existing_cli_full_catalog_behavior() {
    let (_, app, mut req) = fixture("legacy-cli");
    req.clients = Some(vec!["claude-code".into(), "codex".into()]);
    assert!(req.selection.is_none());
    let out = app.plan(&req).unwrap();
    assert_eq!(out.entry_count, 2);
    assert_eq!(out.plans.len(), 2);
    assert!(out.plans.iter().all(|plan| plan.changes.len() == 2));
}
