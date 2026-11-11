use mcp_app::{distribution::DistributionSessions, App, AppError, HostEnv, PlanRequest};
use mcp_core::{adapters, plan::MergeMode, Platform, Scope};
use mcp_io::ApplyReceipt;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const CATALOG: &str = "version: 1\nentries:\n  - {name: alpha, transport: stdio, command: python}\n  - {name: beta, transport: stdio, command: node}\n";
fn write(path: &Path, text: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn fixture() -> (PathBuf, App, PlanRequest) {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "mcphub-distribution-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("project")).unwrap();
    let app = App::with_home(
        root.join("data"),
        HostEnv {
            home: root.clone(),
            appdata: Some(root.join("AppData/Roaming")),
            project_root: None,
        },
        Platform::Windows,
    );
    let mut req = PlanRequest::new(root.join("catalog.yaml"));
    req.selection = Some(BTreeMap::from([
        ("claude-code".into(), vec!["alpha".into()]),
        ("codex".into(), vec!["beta".into()]),
    ]));
    write(&req.catalog_path, CATALOG);
    write(
        &target(&app, "claude-code", req.scope),
        r#"{"theme":"dark","mcpServers":{"keep":{"command":"keep"}}}"#,
    );
    write(
        &target(&app, "codex", req.scope),
        "theme = 'dark'\n[mcp_servers.keep]\ncommand = 'keep'\n",
    );
    (root, app, req)
}
fn target(app: &App, client: &str, scope: Scope) -> PathBuf {
    app.target_path(adapters::by_id(client).unwrap(), scope)
        .unwrap()
}
// 包括目录与符号链接，不跟随链接；证明拒绝时连备份/审计/临时目录也未创建。
fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(path: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_symlink() {
                out.insert(
                    path.clone(),
                    Some(
                        fs::read_link(path)
                            .unwrap()
                            .to_string_lossy()
                            .as_bytes()
                            .to_vec(),
                    ),
                );
            } else if kind.is_dir() {
                out.insert(path.clone(), None);
                visit(&path, out);
            } else {
                out.insert(path.clone(), Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, &mut result);
    result
}
fn reject(
    root: &Path,
    sessions: &mut DistributionSessions,
    app: &App,
    req: &PlanRequest,
    id: &str,
) {
    let before = tree(root);
    let error = sessions
        .commit(app, req, id, false)
        .err()
        .expect("changed preview must be rejected")
        .to_string();
    assert!(error.contains("重新预览"));
    for secret in [
        "your_preview_secret_here",
        "your_external_secret_here",
        "your_resolved_before_here",
        "your_resolved_after_here",
    ] {
        assert!(!error.contains(secret));
    }
    assert_eq!(tree(root), before, "拒绝不能写入目标、备份或审计");
}

#[test]
fn unchanged_preview_writes_exact_selected_pairs_and_is_single_use() {
    let (root, app, req) = fixture();
    let mut sessions = DistributionSessions::default();
    let before = tree(&root);
    let (id, preview) = sessions.preview(&app, &req).unwrap();
    assert_eq!(tree(&root), before, "预览只读");
    assert_eq!(preview.writable(), 2);
    let receipt = sessions.commit(&app, &req, &id, false).unwrap();
    assert_eq!(receipt.written(), 2);
    assert_eq!(receipt.failed(), 0);
    let claude: Value =
        serde_json::from_str(&fs::read_to_string(target(&app, "claude-code", req.scope)).unwrap())
            .unwrap();
    assert_eq!(claude["mcpServers"]["alpha"]["command"], "python");
    assert!(claude["mcpServers"].get("beta").is_none());
    assert_eq!(claude["theme"], "dark");
    assert_eq!(claude["mcpServers"]["keep"]["command"], "keep");
    let codex = fs::read_to_string(target(&app, "codex", req.scope)).unwrap();
    assert!(codex.contains("mcp_servers.beta") && !codex.contains("mcp_servers.alpha"));
    assert!(codex.contains("mcp_servers.keep"));
    let backups = app.backups().unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].files.len(), 2);
    reject(&root, &mut sessions, &app, &req, &id);
}

#[test]
fn catalog_byte_changes_invalid_utf8_and_unselected_entry_changes_block_the_whole_batch() {
    for content in [
        format!("{CATALOG}# comment changed\n").into_bytes(),
        CATALOG
            .replace("command: python", "command: your_external_secret_here")
            .into_bytes(),
        CATALOG.replace("name: beta", "name: renamed").into_bytes(),
        b"entries: your_preview_secret_here".to_vec(),
        vec![0xff, 0xfe],
    ] {
        let (root, app, req) = fixture();
        let mut sessions = DistributionSessions::default();
        let (id, _) = sessions.preview(&app, &req).unwrap();
        write(&req.catalog_path, content);
        reject(&root, &mut sessions, &app, &req, &id);
        write(&req.catalog_path, CATALOG);
        reject(&root, &mut sessions, &app, &req, &id);
    }
    let (root, app, mut req) = fixture();
    req.selection.as_mut().unwrap().remove("codex");
    let mut sessions = DistributionSessions::default();
    let (id, _) = sessions.preview(&app, &req).unwrap();
    write(
        &req.catalog_path,
        CATALOG.replace("command: node", "command: your_external_secret_here"),
    );
    reject(&root, &mut sessions, &app, &req, &id);
}

#[test]
fn missing_catalog_or_target_and_newly_created_targets_are_not_silently_replanned() {
    for kind in [
        "catalog-missing",
        "target-missing",
        "target-created",
        "empty-target-missing",
        "target-directory",
    ] {
        let (root, app, req) = fixture();
        let mut sessions = DistributionSessions::default();
        let path = if kind == "catalog-missing" {
            req.catalog_path.clone()
        } else {
            target(&app, "codex", req.scope)
        };
        if kind == "target-created" {
            fs::rename(&path, path.with_extension("moved")).unwrap();
        }
        if kind == "empty-target-missing" {
            write(&path, "");
        }
        let (id, _) = sessions.preview(&app, &req).unwrap();
        if kind == "target-created" {
            write(&path, "");
        } else {
            fs::rename(&path, path.with_extension("external-copy")).unwrap();
        }
        if kind == "target-directory" {
            fs::create_dir(&path).unwrap();
        }
        reject(&root, &mut sessions, &app, &req, &id);
    }
}

#[test]
fn changing_any_target_including_unchanged_or_blocked_files_prevents_other_writes() {
    for kind in ["changed", "unchanged", "blocked"] {
        let (root, app, req) = fixture();
        let mut sessions = DistributionSessions::default();
        let path = target(&app, "claude-code", req.scope);
        if kind == "unchanged" {
            write(&path, r#"{"mcpServers":{"alpha":{"command":"python","type":"stdio"}}}"#);
        }
        if kind == "blocked" {
            write(&path, "{ invalid your_preview_secret_here");
        }
        let (id, plan) = sessions.preview(&app, &req).unwrap();
        if kind != "changed" {
            assert_eq!(plan.writable(), 1);
        }
        write(
            &path,
            r#"{"external":"your_external_secret_here","mcpServers":{}}"#,
        );
        reject(&root, &mut sessions, &app, &req, &id);
    }
}

#[test]
fn request_tampering_and_context_changes_cannot_reuse_a_preview() {
    for kind in [
        "mode",
        "scope",
        "selection",
        "clients",
        "comments",
        "catalog",
        "home",
        "appdata",
        "project",
        "platform",
        "data",
        "backups",
    ] {
        let (root, mut app, mut req) = fixture();
        let mut sessions = DistributionSessions::default();
        let (id, _) = sessions.preview(&app, &req).unwrap();
        match kind {
            "mode" => req.mode = MergeMode::Replace,
            "scope" => req.scope = Scope::Project,
            "selection" => {
                req.selection
                    .as_mut()
                    .unwrap()
                    .insert("codex".into(), vec!["alpha".into()]);
            }
            "clients" => req.clients = Some(vec!["codex".into()]),
            "comments" => req.allow_comment_strip = true,
            "catalog" => {
                let copy = root.join("copy.yaml");
                fs::copy(&req.catalog_path, &copy).unwrap();
                req.catalog_path = copy;
            }
            "home" => app.env.home = root.join("other-home"),
            "appdata" => app.env.appdata = Some(root.join("other-appdata")),
            "project" => app.env.project_root = Some(root.join("project")),
            "platform" => app.platform = Platform::Linux,
            "data" => app.engine = mcp_io::Engine::new(root.join("other-data")),
            "backups" => {
                app.engine = mcp_io::Engine::new(app.engine.home().to_path_buf())
                    .with_backups_dir(root.join("other-backups"))
            }
            _ => unreachable!(),
        }
        reject(&root, &mut sessions, &app, &req, &id);
    }
}

#[test]
fn software_settings_content_and_existence_are_bound_to_the_preview() {
    for existed in [false, true] {
        let (root, app, req) = fixture();
        let mut sessions = DistributionSessions::default();
        let settings = serde_json::to_string(&app.default_settings()).unwrap();
        if existed {
            write(&app.settings_path(), &settings);
        }
        let (id, _) = sessions.preview(&app, &req).unwrap();
        write(&app.settings_path(), format!("{settings}\n"));
        reject(&root, &mut sessions, &app, &req, &id);
    }
}

#[test]
fn a_new_higher_priority_candidate_does_not_redirect_the_confirmed_write() {
    let (root, mut app, mut req) = fixture();
    app.env.project_root = Some(root.join("project"));
    req.scope = Scope::Project;
    req.selection = Some(BTreeMap::from([("vscode".into(), vec!["alpha".into()])]));
    let portable = root.join("project/.mcp.json");
    write(&portable, "{}");
    let mut sessions = DistributionSessions::default();
    let (id, plan) = sessions.preview(&app, &req).unwrap();
    assert_eq!(Path::new(&plan.plans[0].path), portable);
    write(&root.join("project/.vscode/mcp.json"), "{}");
    reject(&root, &mut sessions, &app, &req, &id);
}

#[test]
fn resolved_environment_changes_including_missing_and_new_values_require_a_new_preview() {
    struct EnvGuard(String);
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(&self.0);
        }
    }
    for (initial, changed) in [
        (
            Some("your_resolved_before_here"),
            Some("your_resolved_after_here"),
        ),
        (None, Some("your_resolved_after_here")),
        (Some("your_resolved_before_here"), None),
    ] {
        let (root, app, req) = fixture();
        let variable = format!(
            "MCPHUB_PREVIEW_{}",
            root.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .replace('-', "_")
        );
        let _guard = EnvGuard(variable.clone());
        if let Some(value) = initial {
            std::env::set_var(&variable, value);
        }
        write(
            &req.catalog_path,
            CATALOG.replace(
                "command: python",
                &format!("command: python, env: {{DEMO: '${{ENV:{variable}}}'}}"),
            ),
        );
        let mut sessions = DistributionSessions::default();
        let (id, _) = sessions.preview(&app, &req).unwrap();
        if let Some(value) = changed {
            std::env::set_var(&variable, value);
        } else {
            std::env::remove_var(&variable);
        }
        reject(&root, &mut sessions, &app, &req, &id);
    }
}

#[test]
fn cancellation_new_previews_failed_previews_and_unknown_ids_are_fail_closed() {
    let (root, app, req) = fixture();
    let mut sessions = DistributionSessions::default();
    reject(&root, &mut sessions, &app, &req, "unknown");
    let (old, _) = sessions.preview(&app, &req).unwrap();
    let (new, _) = sessions.preview(&app, &req).unwrap();
    sessions.cancel(&old);
    reject(&root, &mut sessions, &app, &req, &old);
    assert_eq!(
        sessions.commit(&app, &req, &new, true).unwrap().written(),
        0
    );
    reject(&root, &mut sessions, &app, &req, &new);
    let (id, _) = sessions.preview(&app, &req).unwrap();
    sessions.cancel(&id);
    reject(&root, &mut sessions, &app, &req, &id);
    let (id, _) = sessions.preview(&app, &req).unwrap();
    write(&req.catalog_path, "entries: your_preview_secret_here");
    assert!(sessions.preview(&app, &req).is_err());
    write(&req.catalog_path, CATALOG);
    reject(&root, &mut sessions, &app, &req, &id);
}

#[test]
fn unchanged_shared_file_still_coalesces_and_empty_selection_stays_read_only() {
    let (root, mut app, mut req) = fixture();
    app.env.project_root = Some(root.join("project"));
    req.scope = Scope::Project;
    req.selection = Some(BTreeMap::from([
        ("claude-code".into(), vec!["alpha".into()]),
        ("vscode".into(), vec!["beta".into()]),
    ]));
    write(&root.join("project/.mcp.json"), "{}");
    let mut sessions = DistributionSessions::default();
    let (id, plan) = sessions.preview(&app, &req).unwrap();
    assert_eq!(plan.writable(), 1);
    assert_eq!(plan.plans.len(), 1);
    let receipt = sessions.commit(&app, &req, &id, false).unwrap();
    assert_eq!(receipt.written(), 1);
    assert_eq!(app.backups().unwrap()[0].files.len(), 1);
    req.selection = Some(BTreeMap::new());
    let before = tree(&root);
    let (id, plan) = sessions.preview(&app, &req).unwrap();
    assert!(plan.plans.is_empty());
    assert_eq!(
        sessions.commit(&app, &req, &id, false).unwrap().written(),
        0
    );
    assert_eq!(tree(&root), before);
}

#[test]
fn dry_run_and_oversized_inputs_do_not_create_writes_or_backups() {
    let (root, app, req) = fixture();
    let mut sessions = DistributionSessions::default();
    let before = tree(&root);
    let (id, _) = sessions.preview(&app, &req).unwrap();
    let result: Result<ApplyReceipt, AppError> = sessions.commit(&app, &req, &id, true);
    assert_eq!(result.unwrap().written(), 0);
    assert_eq!(tree(&root), before);
    write(&req.catalog_path, vec![b' '; 4 * 1024 * 1024 + 1]);
    let before = tree(&root);
    assert!(sessions
        .preview(&app, &req)
        .err()
        .unwrap()
        .to_string()
        .contains("4 MiB"));
    assert_eq!(tree(&root), before);
}

#[cfg(unix)]
#[test]
fn catalog_and_target_symlink_redirection_is_rejected_even_for_identical_contents() {
    use std::os::unix::fs::symlink;
    for catalog_link in [true, false] {
        let (root, app, req) = fixture();
        let alias = if catalog_link {
            req.catalog_path.clone()
        } else {
            target(&app, "claude-code", req.scope)
        };
        let first = alias.with_extension("first");
        let second = alias.with_extension("second");
        fs::rename(&alias, &first).unwrap();
        fs::copy(&first, &second).unwrap();
        symlink(&first, &alias).unwrap();
        let mut sessions = DistributionSessions::default();
        let (id, _) = sessions.preview(&app, &req).unwrap();
        fs::rename(&alias, alias.with_extension("old-link")).unwrap();
        symlink(&second, &alias).unwrap();
        reject(&root, &mut sessions, &app, &req, &id);
    }
}
