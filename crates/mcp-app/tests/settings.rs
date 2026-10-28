use mcp_app::{App, HostEnv, PlanRequest};
use mcp_core::{Platform, Scope};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn fixture() -> App {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mcphub-settings-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    App::with_home(
        dir.join(".mcp-hub"),
        HostEnv {
            home: dir.clone(),
            appdata: Some(dir.join("AppData/Roaming")),
            project_root: Some(dir.join("launch-directory")),
        },
        Platform::current(),
    )
}
fn restart(app: &App) -> App {
    App::with_home(
        app.engine.home().to_path_buf(),
        app.env.clone(),
        app.platform,
    )
}

#[test]
fn first_start_defaults_are_absolute_and_read_only() {
    let app = fixture();
    let settings = app.load_settings().unwrap();
    assert_eq!(settings, app.default_settings());
    assert_eq!(settings.default_scope, Scope::User);
    assert!(PathBuf::from(settings.catalog_path).is_absolute());
    assert!(!app.engine.home().exists(), "读取默认设置不创建文件");
}

#[test]
fn settings_survive_restart_and_preserve_original_backup() {
    let app = fixture();
    let mut settings = app.default_settings();
    settings.last_project_dir = Some(app.env.home.join("项目 中文").display().to_string());
    app.save_settings(&settings).unwrap();
    let before = fs::read(app.settings_path()).unwrap();
    settings.backup_dir = app.env.home.join("other-backups").display().to_string();
    settings.default_scope = Scope::Project;
    settings.default_mode = "replace".into();
    app.save_settings(&settings).unwrap();
    assert_eq!(restart(&app).load_settings().unwrap(), settings);
    let backups = app.backups().unwrap();
    let old = backups.iter().find(|b| b.files[0].existed).unwrap();
    assert_eq!(
        fs::read(
            app.engine
                .backups_dir()
                .join(&old.id)
                .join(&old.files[0].stored)
        )
        .unwrap(),
        before
    );
    assert!(
        !PathBuf::from(settings.backup_dir).exists(),
        "保存偏好不迁移或创建目标备份目录"
    );
}

#[test]
fn invalid_settings_never_replace_saved_preferences() {
    let app = fixture();
    let settings = app.default_settings();
    app.save_settings(&settings).unwrap();
    let before = fs::read(app.settings_path()).unwrap();
    let mut cases = Vec::new();
    let mut next = settings.clone();
    next.version = 2;
    cases.push(next);
    let mut next = settings.clone();
    next.default_scope = Scope::Local;
    cases.push(next);
    let mut next = settings.clone();
    next.default_mode = "unknown".into();
    cases.push(next);
    let mut next = settings.clone();
    next.catalog_path = "relative.yaml".into();
    cases.push(next);
    let mut next = settings.clone();
    next.backup_dir = "relative".into();
    cases.push(next);
    let mut next = settings.clone();
    next.last_project_dir = Some("relative".into());
    cases.push(next);
    let mut next = settings.clone();
    next.backup_dir = app.settings_path().display().to_string();
    cases.push(next);
    let mut next = settings.clone();
    next.catalog_path = app.env.home.display().to_string();
    cases.push(next);
    for bad in cases {
        assert!(app.save_settings(&bad).is_err());
        assert_eq!(fs::read(app.settings_path()).unwrap(), before);
    }
}

#[test]
fn corrupt_or_unknown_settings_fail_closed_without_source_text() {
    let app = fixture();
    fs::create_dir_all(app.engine.home()).unwrap();
    for text in [
        "broken your_secret_here",
        "{\"unexpected\":\"your_secret_here\"}",
    ] {
        fs::write(app.settings_path(), text).unwrap();
        let error = app.load_settings().unwrap_err().to_string();
        assert!(!error.contains("your_secret_here"));
        assert!(restart(&app).gui_context(Scope::User, None).is_err());
        assert_eq!(fs::read_to_string(app.settings_path()).unwrap(), text);
    }
}

#[test]
fn project_requires_an_explicit_existing_absolute_directory() {
    let app = fixture();
    fs::create_dir_all(app.env.project_root.as_ref().unwrap()).unwrap();
    for path in [
        None,
        Some(PathBuf::from("relative")),
        Some(app.env.home.join("missing")),
    ] {
        assert!(restart(&app).gui_context(Scope::Project, path).is_err());
    }
    assert_eq!(
        app.env.project_root,
        restart(&app).env.project_root,
        "CLI 上下文不变"
    );
    let chosen = app.env.home.join("chosen 中文目录");
    fs::create_dir_all(&chosen).unwrap();
    let gui = restart(&app)
        .gui_context(Scope::Project, Some(chosen.clone()))
        .unwrap();
    assert_eq!(
        gui.env.project_root.as_ref().unwrap(),
        &fs::canonicalize(chosen).unwrap()
    );
    let global = restart(&app)
        .gui_context(Scope::User, gui.env.project_root)
        .unwrap();
    assert!(global.env.project_root.is_none());
    assert!(global.var_ctx().project_root.is_none());
    assert!(!app.engine.home().exists());
}

#[test]
fn chosen_project_drives_read_plan_apply_and_custom_backups() {
    let app = fixture();
    let chosen = app.env.home.join("chosen project");
    fs::create_dir_all(&chosen).unwrap();
    let mut settings = app.default_settings();
    settings.backup_dir = app.env.home.join("custom backups").display().to_string();
    app.save_settings(&settings).unwrap();
    let catalog = app.env.home.join("catalog.yaml");
    fs::write(&catalog, "version: 1\nentries:\n  - {name: alpha, transport: stdio, command: python}\n  - {name: beta, transport: stdio, command: untouched}\n").unwrap();
    let gui = restart(&app)
        .gui_context(Scope::Project, Some(chosen.clone()))
        .unwrap();
    let config = gui.read_client_config("codex", Scope::Project).unwrap();
    assert!(PathBuf::from(&config.path).starts_with(fs::canonicalize(&chosen).unwrap()));
    assert!(!config.existed);
    let mut req = PlanRequest::new(catalog);
    req.scope = Scope::Project;
    req.selection = Some(BTreeMap::from([("codex".into(), vec!["alpha".into()])]));
    let plan = gui.plan(&req).unwrap();
    assert_eq!(plan.plans.len(), 1);
    assert_eq!(plan.plans[0].path, config.path);
    let receipt = gui.apply(&plan.plans, false).unwrap();
    assert_eq!(receipt.written(), 1);
    assert_eq!(
        gui.read_client_config("codex", Scope::Project)
            .unwrap()
            .entries[0]
            .name,
        "alpha"
    );
    assert_eq!(gui.backups().unwrap().len(), 1);
    assert!(PathBuf::from(&settings.backup_dir)
        .join(receipt.backup_id.unwrap())
        .exists());
    assert!(!app.env.project_root.as_ref().unwrap().exists());
    let global = restart(&app)
        .gui_context(Scope::User, Some(chosen))
        .unwrap();
    assert!(
        !global
            .read_client_config("codex", Scope::User)
            .unwrap()
            .existed
    );
    assert_eq!(global.backups().unwrap().len(), 1);
}

#[test]
fn empty_library_creation_is_valid_and_never_overwrites_existing_file() {
    let app = fixture();
    let path = app.env.home.join("library.yaml");
    app.create_catalog(&path).unwrap();
    assert!(app.load_catalog(&path).unwrap().0.entries.is_empty());
    let before = fs::read(&path).unwrap();
    assert!(app.create_catalog(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(app
        .create_catalog(std::path::Path::new("relative.yaml"))
        .is_err());
}
