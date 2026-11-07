use mcp_app::{
    importing::{ImportAction, ImportChoice, ImportScan, ImportSessions},
    App, HostEnv,
};
use mcp_core::{
    adapters::{self, Adapter},
    document::{ConfigDoc, MergeMode},
    model::{Auth, McpEntry, Platform, Scope, SecretSource},
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const SECRET: &str = "your_secret_here_123";
fn fixture(platform: Platform) -> (App, PathBuf) {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let home = std::env::temp_dir().join(format!(
        "mcphub-import-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&home).unwrap();
    let target = home.join("catalog.yaml");
    let app = App::with_home(
        home.join(".mcp-hub"),
        HostEnv {
            home: home.clone(),
            appdata: Some(home.join("AppData/Roaming")),
            project_root: None,
        },
        platform,
    );
    (app, target)
}
fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn source(app: &App, adapter: &dyn Adapter, scope: Scope, values: &[(&str, Value)]) -> PathBuf {
    let path = app.target_path(adapter, scope).unwrap();
    let mut doc = ConfigDoc::parse(adapter.format_for_path(&path), "").unwrap();
    for (name, value) in values {
        doc.upsert(
            &adapter.root_key_for_path(scope, &path),
            name,
            value,
            MergeMode::Replace,
        )
        .unwrap();
    }
    write(&path, &doc.render());
    path
}
fn choice(scan: &ImportScan, name: &str, action: ImportAction, target: &str) -> ImportChoice {
    let item = scan.items.iter().find(|item| item.name == name).unwrap();
    ImportChoice {
        source_name: name.into(),
        action,
        target_name: target.into(),
        copy_fields: BTreeSet::new(),
        bindings: item
            .fields
            .iter()
            .enumerate()
            .map(|(i, field)| (field.id.clone(), format!("IMPORT_VALUE_{i}")))
            .collect(),
    }
}
fn assert_hidden(value: &impl serde::Serialize) {
    let text = serde_json::to_string(value).unwrap();
    assert!(!text.contains(SECRET));
    assert!(!text.contains("example.invalid"));
    assert!(!text.contains("your_existing_secret_here"));
}

#[test]
fn all_14_adapters_on_three_platforms_import_stdio_and_http_without_secrets() {
    for platform in [Platform::Windows, Platform::Linux, Platform::Macos] {
        for adapter in adapters::all() {
            let (app, target) = fixture(platform);
            let mut local = McpEntry::new_stdio(
                "本地😀",
                "python",
                vec!["-y".into(), "--password".into(), SECRET.into()],
            );
            local.env.insert("PASSWORD".into(), SECRET.into());
            let mut remote = McpEntry::new_http(
                "远程",
                format!("https://user:{SECRET}@example.invalid/mcp?key={SECRET}"),
            );
            remote.auth = Some(Auth::env_ref("EXISTING_AUTH"));
            remote.headers.insert("X-Api-Key".into(), SECRET.into());
            let path = source(
                &app,
                *adapter,
                Scope::User,
                &[
                    (&local.name, adapter.encode_entry(&local).unwrap()),
                    (&remote.name, adapter.encode_entry(&remote).unwrap()),
                ],
            );
            let original = fs::read(&path).unwrap();
            let mut sessions = ImportSessions::default();
            let scan = sessions
                .scan(&app, &target, adapter.id(), Scope::User)
                .unwrap();
            assert!(!target.exists());
            assert!(!app.engine.home().exists());
            assert_hidden(&scan);
            assert!(
                scan.items.iter().all(|item| item.blocked.is_none()),
                "{} {platform:?}",
                adapter.id()
            );
            let choices = scan
                .items
                .iter()
                .map(|item| choice(&scan, &item.name, ImportAction::Add, &item.name))
                .collect::<Vec<_>>();
            let preview = sessions.preview(&app, &scan.scan_id, &choices).unwrap();
            assert_hidden(&preview);
            assert!(!target.exists());
            assert!(!app.engine.home().exists());
            let receipt = sessions.commit(&app, &preview.preview_id).unwrap();
            assert_eq!(receipt.imported, 2);
            let (catalog, _) = app.load_catalog(&target).unwrap();
            assert_eq!(catalog.entries.len(), 2);
            assert_hidden(&catalog);
            assert_eq!(fs::read(&path).unwrap(), original);
            let remote = catalog
                .entries
                .iter()
                .find(|entry| entry.name == "远程")
                .unwrap();
            assert_eq!(
                remote.auth.as_ref().unwrap().env_name(),
                Some("EXISTING_AUTH")
            );
            assert!(catalog
                .entries
                .iter()
                .all(|entry| entry.scope == Some(Scope::User)));
            assert!(
                sessions.commit(&app, &preview.preview_id).is_err(),
                "预览仅能提交一次"
            );
        }
    }
}

#[test]
fn conflicts_require_explicit_actions_and_preserve_unselected_entries_and_backup() {
    let (app, target) = fixture(Platform::current());
    write(&target, "version: 1\nentries:\n  - {name: alpha, transport: stdio, command: old, args: [--password, your_existing_secret_here], overrides: {windows: {command: old-windows}}}\n  - {name: keep, transport: stdio, command: untouched}\n");
    let before = fs::read(&target).unwrap();
    source(
        &app,
        adapters::by_id("codex").unwrap(),
        Scope::User,
        &[
            ("alpha", json!({"command":"python"})),
            ("beta", json!({"command":"node"})),
        ],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions.scan(&app, &target, "codex", Scope::User).unwrap();
    assert!(
        scan.items
            .iter()
            .find(|i| i.name == "alpha")
            .unwrap()
            .conflict
    );
    assert!(sessions
        .preview(
            &app,
            &scan.scan_id,
            &[choice(&scan, "alpha", ImportAction::Add, "alpha")]
        )
        .is_err());
    assert!(sessions
        .preview(
            &app,
            &scan.scan_id,
            &[choice(&scan, "alpha", ImportAction::Rename, "keep")]
        )
        .is_err());
    assert!(sessions
        .preview(
            &app,
            &scan.scan_id,
            &[choice(&scan, "beta", ImportAction::Replace, "beta")]
        )
        .is_err());
    let preview = sessions
        .preview(
            &app,
            &scan.scan_id,
            &[
                choice(&scan, "alpha", ImportAction::Rename, "alpha-copy"),
                choice(&scan, "beta", ImportAction::Add, "beta"),
            ],
        )
        .unwrap();
    assert_hidden(&preview);
    sessions.commit(&app, &preview.preview_id).unwrap();
    let (catalog, _) = app.load_catalog(&target).unwrap();
    assert_eq!(
        catalog
            .entries
            .iter()
            .find(|e| e.name == "alpha")
            .unwrap()
            .command
            .as_deref(),
        Some("old")
    );
    assert_eq!(
        catalog
            .entries
            .iter()
            .find(|e| e.name == "keep")
            .unwrap()
            .command
            .as_deref(),
        Some("untouched")
    );
    let backup = app
        .backups()
        .unwrap()
        .into_iter()
        .find(|backup| backup.files[0].existed)
        .unwrap();
    assert_eq!(
        fs::read(
            app.engine
                .backups_dir()
                .join(&backup.id)
                .join(&backup.files[0].stored)
        )
        .unwrap(),
        before
    );
    let scan = sessions.scan(&app, &target, "codex", Scope::User).unwrap();
    let preview = sessions
        .preview(
            &app,
            &scan.scan_id,
            &[choice(&scan, "alpha", ImportAction::Replace, "alpha")],
        )
        .unwrap();
    assert_hidden(&preview);
    assert!(preview.changes[0].before.is_some());
    sessions.commit(&app, &preview.preview_id).unwrap();
    let (catalog, _) = app.load_catalog(&target).unwrap();
    assert_eq!(
        catalog
            .entries
            .iter()
            .find(|e| e.name == "alpha")
            .unwrap()
            .command
            .as_deref(),
        Some("python")
    );
    assert!(catalog
        .entries
        .iter()
        .find(|e| e.name == "alpha")
        .unwrap()
        .overrides
        .is_empty());
}

#[test]
fn bindings_are_complete_strict_and_never_read_process_env_values() {
    let (app, target) = fixture(Platform::current());
    source(
        &app,
        adapters::by_id("claude-code").unwrap(),
        Scope::User,
        &[(
            "alpha",
            json!({"url":format!("https://user:{SECRET}@example.invalid/mcp"), "headers":{"Authorization":format!("Bearer {SECRET}"),"X-Api-Key":SECRET}}),
        )],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .unwrap();
    assert_eq!(scan.items[0].fields.len(), 3);
    assert_hidden(&scan);
    let valid = choice(&scan, "alpha", ImportAction::Add, "alpha");
    for invalid in [
        "",
        "1INVALID",
        "NOT-VALID",
        "HAS SPACE",
        "NAME\n",
        "${ENV:VALUE}",
        "[已隐藏]",
    ] {
        let mut bad = valid.clone();
        *bad.bindings.values_mut().next().unwrap() = invalid.into();
        assert!(sessions.preview(&app, &scan.scan_id, &[bad]).is_err());
    }
    let mut missing = valid.clone();
    missing.bindings.clear();
    assert!(sessions.preview(&app, &scan.scan_id, &[missing]).is_err());
    let mut extra = valid.clone();
    extra.bindings.insert("/unknown".into(), "EXTRA".into());
    assert!(sessions.preview(&app, &scan.scan_id, &[extra]).is_err());
    let mut existing_env = valid.clone();
    for value in existing_env.bindings.values_mut() {
        *value = "PATH".into();
    }
    let preview = sessions
        .preview(&app, &scan.scan_id, &[existing_env])
        .unwrap();
    assert_hidden(&preview);
    sessions.commit(&app, &preview.preview_id).unwrap();
    let text = fs::read_to_string(target).unwrap();
    assert!(text.contains("${ENV:PATH}"));
    let audit = fs::read_to_string(app.engine.audit_path()).unwrap();
    assert!(!audit.contains(SECRET));
}

#[test]
fn unknown_malformed_or_lossy_fields_are_blocked_individually() {
    let bad = [
        json!({"command":"python","custom":SECRET}),
        json!({"command":"python","args":["safe",123]}),
        json!({"command":"python","env":{"PASSWORD":123}}),
        json!({"command":"python","env":{},"environment":{}}),
        json!({"url":"https://example.invalid","command":"python"}),
        json!({"url":"https://example.invalid","serverUrl":"https://example.invalid"}),
        json!({"url":"https://example.invalid","headers":{"Authorization":format!("Basic {SECRET}")}}),
        json!({"url":"https://example.invalid","headers":{"Authorization":"Bearer a","authorization":"Bearer b"}}),
        json!({"url":"https://example.invalid","headers":{"Authorization":"Bearer a"},"bearer_token_env_var":"VALID"}),
        json!({"command":"python","enabled":"true"}),
        json!({"command":"python","startup_timeout_sec":-1}),
        json!({"command":"python","type":"unknown"}),
        json!({"command":[]}),
        json!({"command":["python","arg"],"args":["other"]}),
        json!({"url":null}),
        json!({"command":"python","headers":{"X-Bad\nHeader":SECRET}}),
    ];
    let (app, target) = fixture(Platform::current());
    let adapter = adapters::by_id("codex").unwrap();
    // JSON 候选允许直接覆盖测试各种字段类型，避免 TOML 自身拒绝 null / 混合数组。
    let path = app.target_path(adapter, Scope::User).unwrap();
    for value in bad {
        let mut doc = ConfigDoc::parse(adapter.spec().format, "").unwrap();
        // TOML 无法表示 null；该情形另在 JSON 来源中检查。
        if value.get("url") == Some(&Value::Null) {
            continue;
        }
        doc.upsert(
            &adapter.root_key(Scope::User),
            "bad",
            &value,
            MergeMode::Replace,
        )
        .unwrap();
        doc.upsert(
            &adapter.root_key(Scope::User),
            "good",
            &json!({"command":"python"}),
            MergeMode::Replace,
        )
        .unwrap();
        write(&path, &doc.render());
        let mut sessions = ImportSessions::default();
        let scan = sessions.scan(&app, &target, "codex", Scope::User).unwrap();
        assert_hidden(&scan);
        assert!(scan
            .items
            .iter()
            .find(|i| i.name == "bad")
            .unwrap()
            .blocked
            .is_some());
        assert!(sessions
            .preview(
                &app,
                &scan.scan_id,
                &[choice(&scan, "bad", ImportAction::Add, "bad")]
            )
            .is_err());
        let preview = sessions
            .preview(
                &app,
                &scan.scan_id,
                &[choice(&scan, "good", ImportAction::Add, "good")],
            )
            .unwrap();
        assert_eq!(preview.changes.len(), 1);
        assert!(!target.exists());
    }
}

#[test]
fn changed_source_catalog_context_and_settings_reject_old_previews() {
    for kind in ["source", "target", "created-target", "settings", "context"] {
        let (mut app, target) = fixture(Platform::current());
        if kind != "created-target" {
            write(&target, "version: 1\nentries: []\n");
        }
        let path = source(
            &app,
            adapters::by_id("codex").unwrap(),
            Scope::User,
            &[("alpha", json!({"command":"python"}))],
        );
        let mut sessions = ImportSessions::default();
        let scan = sessions.scan(&app, &target, "codex", Scope::User).unwrap();
        let preview = sessions
            .preview(
                &app,
                &scan.scan_id,
                &[choice(&scan, "alpha", ImportAction::Add, "alpha")],
            )
            .unwrap();
        match kind {
            "source" => write(&path, "[mcp_servers.changed]\ncommand = 'node'\n"),
            "target" | "created-target" => write(
                &target,
                "version: 1\nentries: [{name: external, transport: stdio, command: node}]\n",
            ),
            "settings" => {
                app.save_settings(&app.default_settings()).unwrap();
            }
            "context" => app.env.project_root = Some(app.env.home.join("different")),
            _ => unreachable!(),
        }
        let before = fs::read(&target).ok();
        assert!(sessions.commit(&app, &preview.preview_id).is_err());
        assert_eq!(fs::read(&target).ok(), before);
    }
}

#[test]
fn cancel_new_preview_failed_preview_and_empty_selection_never_write() {
    let (app, target) = fixture(Platform::current());
    source(
        &app,
        adapters::by_id("codex").unwrap(),
        Scope::User,
        &[("alpha", json!({"command":"python"}))],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions.scan(&app, &target, "codex", Scope::User).unwrap();
    assert!(sessions.preview(&app, &scan.scan_id, &[]).is_err());
    assert!(sessions
        .preview(
            &app,
            &scan.scan_id,
            &[choice(&scan, "alpha", ImportAction::Skip, "alpha")]
        )
        .is_err());
    let choices = vec![choice(&scan, "alpha", ImportAction::Add, "alpha")];
    let old = sessions.preview(&app, &scan.scan_id, &choices).unwrap();
    let new = sessions.preview(&app, &scan.scan_id, &choices).unwrap();
    assert!(sessions.commit(&app, &old.preview_id).is_err());
    let mut unknown = choices[0].clone();
    unknown.source_name = "unknown".into();
    assert!(sessions.preview(&app, &scan.scan_id, &[unknown]).is_err());
    assert!(sessions.commit(&app, &new.preview_id).is_err());
    let new = sessions.preview(&app, &scan.scan_id, &choices).unwrap();
    sessions.cancel(&scan.scan_id);
    assert!(sessions.commit(&app, &new.preview_id).is_err());
    assert!(!target.exists());
    assert!(!app.engine.home().exists());
}

#[test]
fn duplicate_sources_or_targets_cannot_overwrite_each_other() {
    let (app, target) = fixture(Platform::current());
    source(
        &app,
        adapters::by_id("codex").unwrap(),
        Scope::User,
        &[
            ("alpha", json!({"command":"python"})),
            ("beta", json!({"command":"node"})),
        ],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions.scan(&app, &target, "codex", Scope::User).unwrap();
    let same = choice(&scan, "alpha", ImportAction::Add, "alpha");
    assert!(sessions
        .preview(&app, &scan.scan_id, &[same.clone(), same])
        .is_err());
    assert!(sessions
        .preview(
            &app,
            &scan.scan_id,
            &[
                choice(&scan, "alpha", ImportAction::Rename, "new"),
                choice(&scan, "beta", ImportAction::Rename, "new")
            ]
        )
        .is_err());
    assert!(!target.exists());
}

#[test]
fn project_scope_is_fixed_and_global_source_is_not_used() {
    let (mut app, target) = fixture(Platform::current());
    let project = app.env.home.join("项目");
    fs::create_dir_all(&project).unwrap();
    app.env.project_root = Some(project.clone());
    source(
        &app,
        adapters::by_id("codex").unwrap(),
        Scope::User,
        &[("global", json!({"command":"node"}))],
    );
    let path = source(
        &app,
        adapters::by_id("codex").unwrap(),
        Scope::Project,
        &[(
            "project",
            json!({"command":"python","args":["${PROJECT_ROOT}","${HOME}","${env:EXISTING}"]}),
        )],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "codex", Scope::Project)
        .unwrap();
    assert_eq!(scan.items.len(), 1);
    assert_eq!(scan.items[0].name, "project");
    assert!(scan.items[0].fields.is_empty());
    let preview = sessions
        .preview(
            &app,
            &scan.scan_id,
            &[choice(&scan, "project", ImportAction::Add, "project")],
        )
        .unwrap();
    sessions.commit(&app, &preview.preview_id).unwrap();
    let (catalog, _) = app.load_catalog(&target).unwrap();
    assert_eq!(catalog.entries[0].scope, Some(Scope::Project));
    assert_eq!(
        catalog.entries[0].args,
        vec!["${PROJECT_ROOT}", "${HOME}", "${ENV:EXISTING}"]
    );
    assert!(path.starts_with(project));
}

#[test]
fn invalid_destinations_and_corrupt_inputs_do_not_write_or_expose_source() {
    let (app, target) = fixture(Platform::current());
    let adapter = adapters::by_id("claude-code").unwrap();
    let path = source(
        &app,
        adapter,
        Scope::User,
        &[("alpha", json!({"command":"python"}))],
    );
    let mut sessions = ImportSessions::default();
    for bad in [
        PathBuf::from("relative.yaml"),
        path.clone(),
        app.settings_path(),
        app.engine.audit_path(),
    ] {
        assert!(sessions
            .scan(&app, &bad, "claude-code", Scope::User)
            .is_err());
    }
    write(&target, "broken: your_secret_here_123");
    assert!(sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .is_err());
    let before = fs::read(&target).unwrap();
    write(&path, "{your_secret_here_123");
    let error = sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains(SECRET));
    assert_eq!(fs::read(&target).unwrap(), before);
    assert!(sessions
        .scan(&app, &target, "unknown", Scope::User)
        .is_err());
    assert!(sessions
        .scan(&app, &target, "claude-code", Scope::Local)
        .is_err());
}

#[test]
fn jsonc_source_and_json_target_keep_unicode_and_source_comments() {
    let (app, _) = fixture(Platform::current());
    let target = app.env.home.join("catalog.json");
    let adapter = adapters::by_id("opencode").unwrap();
    let path = app.target_path(adapter, Scope::User).unwrap();
    let raw = "\u{feff}{ // 原始注释😀\n\"mcp\":{\"servers\":{\"中文😀\":{\"type\":\"local\",\"command\":[\"python\",\"--password\",\"your_secret_here_123\"],},},},}";
    write(&path, raw);
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "opencode", Scope::User)
        .unwrap();
    assert_hidden(&scan);
    let preview = sessions
        .preview(
            &app,
            &scan.scan_id,
            &[choice(&scan, "中文😀", ImportAction::Add, "中文😀")],
        )
        .unwrap();
    sessions.commit(&app, &preview.preview_id).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), raw);
    let value: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(value["entries"][0]["name"], "中文😀");
    assert_hidden(&value);
}

#[test]
fn backup_failure_preserves_catalog_and_audit_failure_is_a_warning() {
    for audit_failure in [false, true] {
        let (mut app, target) = fixture(Platform::current());
        write(&target, "version: 1\nentries: []\n");
        let before = fs::read(&target).unwrap();
        source(
            &app,
            adapters::by_id("codex").unwrap(),
            Scope::User,
            &[("alpha", json!({"command":"python"}))],
        );
        if audit_failure {
            fs::create_dir_all(app.engine.audit_path()).unwrap();
        } else {
            let blocker = app.env.home.join("backup-is-a-file");
            write(&blocker, "your_backup_placeholder_here");
            app.engine = app.engine.with_backups_dir(blocker);
        }
        let mut sessions = ImportSessions::default();
        let scan = sessions.scan(&app, &target, "codex", Scope::User).unwrap();
        let preview = sessions
            .preview(
                &app,
                &scan.scan_id,
                &[choice(&scan, "alpha", ImportAction::Add, "alpha")],
            )
            .unwrap();
        let result = sessions.commit(&app, &preview.preview_id);
        if audit_failure {
            let result = result.unwrap();
            assert_eq!(result.imported, 1);
            assert!(!result.warnings.is_empty());
        } else {
            assert!(result.is_err());
            assert_eq!(fs::read(target).unwrap(), before);
        }
    }
}

#[test]
fn higher_priority_source_appearing_after_scan_invalidates_preview() {
    let (app, target) = fixture(Platform::current());
    let templates = adapters::paths::candidates("grok", Scope::User, app.platform);
    let paths = app.env.expand_all(&templates, app.platform);
    write(
        &paths[1],
        r#"{"mcp_servers":{"alpha":{"command":"python"}}}"#,
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions.scan(&app, &target, "grok", Scope::User).unwrap();
    let preview = sessions
        .preview(
            &app,
            &scan.scan_id,
            &[choice(&scan, "alpha", ImportAction::Add, "alpha")],
        )
        .unwrap();
    write(&paths[0], "[mcp_servers.other]\ncommand = 'node'\n");
    assert!(sessions.commit(&app, &preview.preview_id).is_err());
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn destination_symlink_to_source_is_rejected() {
    let (app, target) = fixture(Platform::current());
    let path = source(
        &app,
        adapters::by_id("claude-code").unwrap(),
        Scope::User,
        &[("alpha", json!({"command":"python"}))],
    );
    std::os::unix::fs::symlink(&path, &target).unwrap();
    assert!(ImportSessions::default()
        .scan(&app, &target, "claude-code", Scope::User)
        .is_err());
}

#[test]
fn other_agents_empty_or_missing_config_paths_are_never_used_as_catalogs() {
    let (mut app, _) = fixture(Platform::current());
    let project = app.env.home.join("project");
    fs::create_dir_all(&project).unwrap();
    app.env.project_root = Some(project);
    source(
        &app,
        adapters::by_id("codex").unwrap(),
        Scope::User,
        &[("alpha", json!({"command":"python"}))],
    );
    let mut sessions = ImportSessions::default();
    for scope in [Scope::User, Scope::Project] {
        let other = app
            .target_path(adapters::by_id("claude-code").unwrap(), scope)
            .unwrap();
        assert!(sessions.scan(&app, &other, "codex", Scope::User).is_err());
        assert!(!other.exists());
        write(&other, "{}");
        assert!(sessions.scan(&app, &other, "codex", Scope::User).is_err());
        assert_eq!(fs::read_to_string(other).unwrap(), "{}");
    }
}

#[test]
fn null_and_non_object_json_entries_are_blocked_without_losing_good_items() {
    let (app, target) = fixture(Platform::current());
    let path = app
        .target_path(adapters::by_id("claude-code").unwrap(), Scope::User)
        .unwrap();
    write(
        &path,
        r#"{"mcpServers":{"bad-null":{"url":null},"bad-object":42,"good":{"command":"python"}}}"#,
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .unwrap();
    assert!(
        scan.items
            .iter()
            .filter(|item| item.blocked.is_some())
            .count()
            == 2
    );
    let preview = sessions
        .preview(
            &app,
            &scan.scan_id,
            &[choice(&scan, "good", ImportAction::Add, "good")],
        )
        .unwrap();
    assert_eq!(preview.changes.len(), 1);
    assert!(!target.exists());
}

#[test]
fn ordinary_import_fields_need_no_bindings_and_keep_their_original_values() {
    let (app, target) = fixture(Platform::current());
    let raw = json!({"command":"C:/Program Files/nodejs/npx.cmd", "args":["-y","@scope/mcp","C:/My Documents","--port","8080"], "cwd":"C:/My Projects/demo", "env":{"NODE_ENV":"production","PATH":"C:/Tools"}});
    let path = source(
        &app,
        adapters::by_id("claude-code").unwrap(),
        Scope::User,
        &[("local", raw)],
    );
    let original = fs::read(&path).unwrap();
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .unwrap();
    assert!(
        scan.items[0].fields.is_empty(),
        "ordinary values must not require environment bindings"
    );
    let choice = ImportChoice {
        source_name: "local".into(),
        action: ImportAction::Add,
        target_name: "local".into(),
        copy_fields: BTreeSet::new(),
        bindings: BTreeMap::new(),
    };
    let preview = sessions.preview(&app, &scan.scan_id, &[choice]).unwrap();
    assert_eq!(preview.changes[0].after["args"][1], "@scope/mcp");
    sessions.commit(&app, &preview.preview_id).unwrap();
    let (catalog, _) = app.load_catalog(&target).unwrap();
    assert_eq!(
        catalog.entries[0].command.as_deref(),
        Some("C:/Program Files/nodejs/npx.cmd")
    );
    assert_eq!(
        catalog.entries[0].args,
        vec!["-y", "@scope/mcp", "C:/My Documents", "--port", "8080"]
    );
    assert_eq!(
        catalog.entries[0].cwd.as_deref(),
        Some("C:/My Projects/demo")
    );
    assert_eq!(catalog.entries[0].env["PATH"], "C:/Tools");
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn mixed_import_maps_only_credentials_and_keeps_normal_remote_settings() {
    let (app, target) = fixture(Platform::current());
    source(
        &app,
        adapters::by_id("claude-code").unwrap(),
        Scope::User,
        &[(
            "remote",
            json!({"url":"https://example.invalid/mcp?project=demo", "headers":{"Accept":"application/json","X-Api-Key":"your_header_key_here"}, "env":{"API_KEY":"${ENV:EXISTING_KEY}","NODE_ENV":"production"}}),
        )],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .unwrap();
    let ids = scan.items[0]
        .fields
        .iter()
        .map(|f| f.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["/headers/X-Api-Key"]);
    let preview = sessions
        .preview(
            &app,
            &scan.scan_id,
            &[ImportChoice {
                source_name: "remote".into(),
                action: ImportAction::Add,
                target_name: "remote".into(),
                copy_fields: BTreeSet::new(),
                bindings: BTreeMap::from([(
                    "/headers/X-Api-Key".into(),
                    "IMPORTED_API_KEY".into(),
                )]),
            }],
        )
        .unwrap();
    assert_eq!(
        preview.changes[0].after["url"],
        "https://example.invalid/mcp?project=demo"
    );
    assert_eq!(
        preview.changes[0].after["headers"]["Accept"],
        "application/json"
    );
    assert!(!serde_json::to_string(&preview)
        .unwrap()
        .contains("your_header_key_here"));
    sessions.commit(&app, &preview.preview_id).unwrap();
    let (catalog, _) = app.load_catalog(&target).unwrap();
    assert_eq!(
        catalog.entries[0].headers["X-Api-Key"],
        "${ENV:IMPORTED_API_KEY}"
    );
    assert_eq!(catalog.entries[0].env["API_KEY"], "${ENV:EXISTING_KEY}");
    assert_eq!(catalog.entries[0].env["NODE_ENV"], "production");
}

#[test]
fn sensitive_argument_bindings_keep_inline_flags_and_other_arguments() {
    let (app, target) = fixture(Platform::current());
    source(
        &app,
        adapters::by_id("claude-code").unwrap(),
        Scope::User,
        &[(
            "local",
            json!({"command":"npx", "args":["@scope/mcp","--token=your_token_here","--password","your_password_here","--port=8080"]}),
        )],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .unwrap();
    assert_eq!(
        scan.items[0]
            .fields
            .iter()
            .map(|f| f.id.as_str())
            .collect::<Vec<_>>(),
        vec!["/args/1", "/args/3"]
    );
    let preview = sessions
        .preview(
            &app,
            &scan.scan_id,
            &[ImportChoice {
                source_name: "local".into(),
                action: ImportAction::Add,
                target_name: "local".into(),
                copy_fields: BTreeSet::new(),
                bindings: BTreeMap::from([
                    ("/args/1".into(), "MY_TOKEN".into()),
                    ("/args/3".into(), "MY_PASSWORD".into()),
                ]),
            }],
        )
        .unwrap();
    assert_eq!(
        preview.changes[0].after["args"],
        json!([
            "@scope/mcp",
            "--token=${ENV:MY_TOKEN}",
            "--password",
            "${ENV:MY_PASSWORD}",
            "--port=8080"
        ])
    );
    let serialized = serde_json::to_string(&preview).unwrap();
    assert!(!serialized.contains("your_token_here") && !serialized.contains("your_password_here"));
    sessions.commit(&app, &preview.preview_id).unwrap();
    let (catalog, _) = app.load_catalog(&target).unwrap();
    assert_eq!(catalog.entries[0].args[1], "--token=${ENV:MY_TOKEN}");
}

#[test]
fn mixed_client_variable_references_are_normalized_without_mapping_or_resolution() {
    let (app, target) = fixture(Platform::current());
    source(
        &app,
        adapters::by_id("opencode").unwrap(),
        Scope::User,
        &[
            (
                "local",
                json!({"type":"local","command":["python","--root=${env:ROOT}/tools","{env:ROOT}/data"],"cwd":"${env:ROOT}/work","environment":{"CONFIG":"{env:ROOT}/cfg"}}),
            ),
            (
                "remote",
                json!({"type":"remote", "url":"https://example.invalid/{env:HOST}/mcp"}),
            ),
        ],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "opencode", Scope::User)
        .unwrap();
    assert!(scan
        .items
        .iter()
        .all(|i| i.fields.is_empty() && i.blocked.is_none()));
    let choices = scan
        .items
        .iter()
        .map(|i| choice(&scan, &i.name, ImportAction::Add, &i.name))
        .collect::<Vec<_>>();
    let preview = sessions.preview(&app, &scan.scan_id, &choices).unwrap();
    sessions.commit(&app, &preview.preview_id).unwrap();
    let (catalog, _) = app.load_catalog(&target).unwrap();
    let local = catalog.entries.iter().find(|e| e.name == "local").unwrap();
    assert_eq!(
        local.args,
        vec!["--root=${ENV:ROOT}/tools", "${ENV:ROOT}/data"]
    );
    assert_eq!(local.cwd.as_deref(), Some("${ENV:ROOT}/work"));
    assert_eq!(local.env["CONFIG"], "${ENV:ROOT}/cfg");
    assert_eq!(
        catalog
            .entries
            .iter()
            .find(|e| e.name == "remote")
            .unwrap()
            .url
            .as_deref(),
        Some("https://example.invalid/${ENV:HOST}/mcp")
    );
}

#[test]
fn a_password_value_that_looks_like_an_inline_flag_is_mapped_in_full() {
    let (app, target) = fixture(Platform::current());
    source(
        &app,
        adapters::by_id("claude-code").unwrap(),
        Scope::User,
        &[(
            "local",
            json!({"command":"python", "args":["--password","-your_password_prefix_here=your_password_suffix_here"]}),
        )],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .unwrap();
    let preview = sessions
        .preview(
            &app,
            &scan.scan_id,
            &[choice(&scan, "local", ImportAction::Add, "local")],
        )
        .unwrap();
    let text = serde_json::to_string(&preview).unwrap();
    assert!(
        !text.contains("your_password_prefix_here") && !text.contains("your_password_suffix_here")
    );
    assert_eq!(preview.changes[0].after["args"][1], "${ENV:IMPORT_VALUE_0}");
}

#[test]
fn password_assignments_matching_scheme_names_still_require_mapping() {
    let (app, target) = fixture(Platform::current());
    source(
        &app,
        adapters::by_id("claude-code").unwrap(),
        Scope::User,
        &[(
            "local",
            json!({"command":"python","args":["PASSWORD=basic","--password","Bearer","Basic mode", "--password=${ENV:PASS}:your_password_suffix_here", "-Dservice.apiKey=your_api_key_here"]}),
        )],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .unwrap();
    assert_eq!(
        scan.items[0]
            .fields
            .iter()
            .map(|f| f.id.as_str())
            .collect::<Vec<_>>(),
        vec!["/args/0", "/args/2", "/args/4", "/args/5"]
    );
}

fn copy_all(scan: &ImportScan, name: &str) -> ImportChoice {
    let item = scan.items.iter().find(|item| item.name == name).unwrap();
    ImportChoice {
        source_name: name.into(),
        action: ImportAction::Add,
        target_name: name.into(),
        bindings: BTreeMap::new(),
        copy_fields: item.fields.iter().map(|field| field.id.clone()).collect(),
    }
}

#[test]
fn copied_credentials_are_written_verbatim_but_hidden_in_scan_and_preview() {
    let (app, target) = fixture(Platform::current());
    let url = format!("https://user:{SECRET}@example.invalid/mcp?key={SECRET}");
    source(
        &app,
        adapters::by_id("claude-code").unwrap(),
        Scope::User,
        &[
            (
                "remote",
                json!({"url":url, "headers":{"Authorization":format!("Bearer {SECRET}"),"X-Api-Key":SECRET,"Accept":"application/json"}}),
            ),
            (
                "local",
                json!({"command":"npx", "args":["@scope/mcp",format!("--token={SECRET}"),"--password",SECRET,"--port=8080"], "env":{"API_KEY":SECRET,"NODE_ENV":"production"}}),
            ),
        ],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .unwrap();
    assert!(!serde_json::to_string(&scan).unwrap().contains(SECRET));
    let choices = vec![copy_all(&scan, "local"), copy_all(&scan, "remote")];
    assert!(choices.iter().all(|c| !c.copy_fields.is_empty()));
    let preview = sessions.preview(&app, &scan.scan_id, &choices).unwrap();
    let shown = serde_json::to_string(&preview).unwrap();
    assert!(!shown.contains(SECRET), "预览不得包含原样复制的凭据");
    assert!(shown.contains("[已隐藏]"));
    sessions.commit(&app, &preview.preview_id).unwrap();

    let (catalog, _) = app.load_catalog(&target).unwrap();
    let local = catalog.entries.iter().find(|e| e.name == "local").unwrap();
    assert_eq!(
        local.args,
        vec![
            "@scope/mcp".to_string(),
            format!("--token={SECRET}"),
            "--password".into(),
            SECRET.into(),
            "--port=8080".into()
        ]
    );
    assert_eq!(local.env["API_KEY"], SECRET);
    assert_eq!(local.env["NODE_ENV"], "production");
    let remote = catalog.entries.iter().find(|e| e.name == "remote").unwrap();
    assert_eq!(remote.url.as_deref(), Some(url.as_str()));
    assert_eq!(remote.headers["X-Api-Key"], SECRET);
    assert_eq!(remote.headers["Accept"], "application/json");
    assert_eq!(
        remote.auth,
        Some(Auth::Bearer {
            token: SecretSource::Literal(SECRET.into())
        })
    );
    let audit = fs::read_to_string(app.engine.audit_path()).unwrap();
    assert!(!audit.contains(SECRET), "审计日志不得记录凭据");
}

#[test]
fn copy_and_mapping_can_be_mixed_per_field() {
    let (app, target) = fixture(Platform::current());
    source(
        &app,
        adapters::by_id("claude-code").unwrap(),
        Scope::User,
        &[(
            "remote",
            json!({"url":"https://example.invalid/mcp", "headers":{"Authorization":format!("Bearer {SECRET}"),"X-Api-Key":SECRET}}),
        )],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .unwrap();
    let choice = ImportChoice {
        source_name: "remote".into(),
        action: ImportAction::Add,
        target_name: "remote".into(),
        bindings: BTreeMap::from([("/headers/X-Api-Key".into(), "MY_API_KEY".into())]),
        copy_fields: BTreeSet::from(["/auth/token_env".to_string()]),
    };
    let preview = sessions.preview(&app, &scan.scan_id, &[choice]).unwrap();
    assert_eq!(preview.changes[0].after["auth"]["token"], "[已隐藏]");
    assert_eq!(
        preview.changes[0].after["headers"]["X-Api-Key"],
        "${ENV:MY_API_KEY}"
    );
    assert!(!serde_json::to_string(&preview).unwrap().contains(SECRET));
    sessions.commit(&app, &preview.preview_id).unwrap();
    let (catalog, _) = app.load_catalog(&target).unwrap();
    assert_eq!(catalog.entries[0].headers["X-Api-Key"], "${ENV:MY_API_KEY}");
    assert_eq!(
        catalog.entries[0].auth,
        Some(Auth::Bearer {
            token: SecretSource::Literal(SECRET.into())
        })
    );
}

#[test]
fn each_credential_field_needs_exactly_one_choice() {
    let (app, target) = fixture(Platform::current());
    source(
        &app,
        adapters::by_id("claude-code").unwrap(),
        Scope::User,
        &[(
            "local",
            json!({"command":"python", "env":{"API_KEY":SECRET,"PASSWORD":SECRET}}),
        )],
    );
    let mut sessions = ImportSessions::default();
    let scan = sessions
        .scan(&app, &target, "claude-code", Scope::User)
        .unwrap();
    let valid = copy_all(&scan, "local");
    assert_eq!(valid.copy_fields.len(), 2);

    let mut missing = valid.clone();
    missing.copy_fields.pop_first();
    assert!(sessions.preview(&app, &scan.scan_id, &[missing]).is_err());

    let mut both = valid.clone();
    let id = both.copy_fields.first().unwrap().clone();
    both.bindings.insert(id, "MY_VALUE".into());
    assert!(sessions.preview(&app, &scan.scan_id, &[both]).is_err());

    let mut unknown = valid.clone();
    unknown.copy_fields.pop_first();
    unknown.copy_fields.insert("/env/UNKNOWN".into());
    assert!(sessions.preview(&app, &scan.scan_id, &[unknown]).is_err());

    // 普通字段不能借“原样复制”绕过校验。
    let mut ordinary = valid.clone();
    ordinary.copy_fields.insert("/command".into());
    assert!(sessions.preview(&app, &scan.scan_id, &[ordinary]).is_err());

    assert!(sessions.preview(&app, &scan.scan_id, &[valid]).is_ok());
    assert!(!target.exists());
}
