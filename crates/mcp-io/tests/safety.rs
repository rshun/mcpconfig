use mcp_core::{
    adapters,
    plan::{plan_file, MergeMode},
    McpEntry, Scope,
};
use mcp_io::{ApplyOptions, Engine, OutcomeStatus};
use std::{fs, path::PathBuf, sync::Arc};

fn fixture() -> (PathBuf, Engine) {
    let root = std::env::temp_dir().join(format!(
        "mcphub-io-safety-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let engine = Engine::new(root.join("data"));
    (root, engine)
}

fn plan(path: &std::path::Path, original: Option<&str>) -> mcp_core::plan::FilePlan {
    plan_file(
        adapters::by_id("claude-code").unwrap(),
        path.display().to_string(),
        Scope::User,
        original,
        &[McpEntry::new_stdio("new", "python", vec![])],
        MergeMode::Merge,
        false,
    )
    .unwrap()
}

#[test]
fn concurrent_backups_have_unique_ids_and_preserve_every_original() {
    let (root, engine) = fixture();
    let engine = Arc::new(engine);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let engine = engine.clone();
            let barrier = barrier.clone();
            let target = root.join(format!("{i}.json"));
            let original = format!("{{\"original\":{i}}}");
            fs::write(&target, &original).unwrap();
            std::thread::spawn(move || {
                barrier.wait();
                let receipt = engine
                    .apply(&[plan(&target, Some(&original))], &ApplyOptions::default())
                    .unwrap();
                assert_eq!(receipt.written(), 1);
                assert_eq!(receipt.failed(), 0);
                let id = receipt.backup_id.unwrap();
                let backup: mcp_io::BackupManifest = serde_json::from_str(
                    &fs::read_to_string(engine.backups_dir().join(&id).join("manifest.json"))
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(
                    fs::read_to_string(
                        engine.backups_dir().join(&id).join(&backup.files[0].stored)
                    )
                    .unwrap(),
                    original
                );
                id
            })
        })
        .collect();
    let ids: std::collections::BTreeSet<_> =
        handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(ids.len(), 8);
    assert_eq!(engine.list_backups().unwrap().len(), 8);
}

#[test]
fn failed_catalog_backup_leaves_original_untouched() {
    let (root, _) = fixture();
    let path = root.join("catalog.yaml");
    let old = "version: 1\nentries: []\n";
    fs::write(&path, old).unwrap();
    let bad_home = root.join("not-a-directory");
    fs::write(&bad_home, "block backup creation").unwrap();
    let result = Engine::new(bad_home).save_catalog(
        &path,
        "version: 1\nentries: [{name: a, transport: stdio, command: python}]\n",
    );
    assert!(result.is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), old);
}

#[test]
fn invalid_catalog_is_rejected_before_backup_or_overwrite() {
    let (root, engine) = fixture();
    let target = root.join("catalog.yaml");
    fs::write(&target, "original").unwrap();
    assert!(engine
        .save_catalog(
            &target,
            "version: 1\nentries: [{name: a, transport: stdio}]\n"
        )
        .is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "original");
    assert!(!engine.backups_dir().exists());
}

#[test]
fn audit_failure_keeps_successful_write_and_does_not_claim_rollback() {
    let (root, engine) = fixture();
    fs::create_dir_all(engine.audit_path()).unwrap();
    let target = root.join("config.json");
    let receipt = engine
        .apply(&[plan(&target, None)], &ApplyOptions::default())
        .unwrap();
    assert_eq!(receipt.written(), 1);
    assert_eq!(receipt.failed(), 1);
    assert_eq!(
        receipt.outcomes.last().unwrap().status,
        OutcomeStatus::AuditFailed
    );
    assert!(receipt
        .outcomes
        .last()
        .unwrap()
        .message
        .as_deref()
        .unwrap()
        .contains("未回滚"));
    assert!(fs::read_to_string(target).unwrap().contains("new"));
}

#[test]
fn duplicate_targets_and_stale_plans_never_overwrite_current_content() {
    let (root, engine) = fixture();
    let target = root.join("config.json");
    let original = "{}";
    fs::write(&target, original).unwrap();
    let p = plan(&target, Some(original));
    assert!(engine
        .apply(&[p.clone(), p.clone()], &ApplyOptions::default())
        .is_err());
    assert!(!engine.backups_dir().exists());
    let external = "{\"external\":true}";
    fs::write(&target, external).unwrap();
    let receipt = engine.apply(&[p], &ApplyOptions::default()).unwrap();
    assert_eq!(receipt.written(), 0);
    assert_eq!(receipt.blocked(), 1);
    assert_eq!(fs::read_to_string(target).unwrap(), external);
}

#[test]
fn verification_error_restores_original_and_does_not_leak_parser_input() {
    let (root, engine) = fixture();
    let target = root.join("config.json");
    let original = "{\"keep\":true}";
    fs::write(&target, original).unwrap();
    let mut p = plan(&target, Some(original));
    p.new_content = "{your_secret_here invalid".into();
    let receipt = engine.apply(&[p], &ApplyOptions::default()).unwrap();
    assert_eq!(receipt.outcomes[0].status, OutcomeStatus::Failed);
    assert!(receipt.outcomes[0]
        .message
        .as_deref()
        .unwrap()
        .contains("已恢复"));
    assert_eq!(fs::read_to_string(target).unwrap(), original);
    assert!(!fs::read_to_string(engine.audit_path())
        .unwrap()
        .contains("your_secret_here"));
}
