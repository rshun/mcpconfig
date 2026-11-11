use mcp_app::{App, HostEnv};
use mcp_core::adapters::{self, Format};
use mcp_core::document::{ConfigDoc, MergeMode};
use mcp_core::{Platform, Scope};
use serde_json::json;
use std::{fs, path::Path};

fn fixture() -> App {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!(
        "mcphub-inspect-{}-{}-{serial}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&home).unwrap();
    App::with_home(
        home.join(".mcp-hub"),
        HostEnv {
            home: home.clone(),
            appdata: Some(home.join("AppData/Roaming")),
            project_root: Some(home.join("project")),
        },
        Platform::Windows,
    )
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn reads_all_14_adapters_on_three_platforms_without_writes_or_catalog() {
    for platform in [Platform::Windows, Platform::Linux, Platform::Macos] {
        for adapter in adapters::all() {
            let mut app = fixture();
            app.platform = platform;
            let path = app.target_path(*adapter, Scope::User).unwrap();
            let mut doc = ConfigDoc::parse(adapter.spec().format, "").unwrap();
            doc.upsert(&adapter.root_key(Scope::User), "中文服务😀", &json!({
                "command": "python", "args": ["your_secret_here"], "env": {"API_KEY": "your_api_key_here"},
            }), MergeMode::Merge).unwrap();
            write(&path, &doc.render());
            let before = fs::read(&path).unwrap();
            let config = app.read_client_config(adapter.id(), Scope::User).unwrap();
            assert!(config.existed);
            assert_eq!(config.entries.len(), 1, "{} {platform:?}", adapter.id());
            assert_eq!(config.entries[0].name, "中文服务😀");
            assert_eq!(config.entries[0].config["command"], "python");
            assert_eq!(config.entries[0].config["env"]["API_KEY"], "[已隐藏]");
            assert_eq!(before, fs::read(&path).unwrap());
            assert!(
                !app.engine.home().exists(),
                "读取不能产生备份、审计或其他写入"
            );
        }
    }
}

#[test]
fn missing_empty_and_invalid_container_are_distinct() {
    let app = fixture();
    let missing = app.read_client_config("claude-code", Scope::User).unwrap();
    assert!(!missing.existed);
    assert!(!Path::new(&missing.path).exists());
    write(
        Path::new(&missing.path),
        r#"{"other":{"field":"your_secret_here"}}"#,
    );
    let empty = app.read_client_config("claude-code", Scope::User).unwrap();
    assert!(empty.existed);
    assert!(empty.entries.is_empty());
    for broken in [
        r#"{"mcpServers":[]}"#,
        r#"{"mcpServers":{"server":"your_secret_here"}}"#,
    ] {
        write(Path::new(&missing.path), broken);
        let error = app
            .read_client_config("claude-code", Scope::User)
            .unwrap_err()
            .to_string();
        assert!(error.contains("结构无效"));
        assert!(!error.contains("your_secret_here"));
    }
}

#[test]
fn parse_and_io_errors_do_not_include_source_text() {
    let app = fixture();
    let adapter = adapters::by_id("codex").unwrap();
    let path = app.target_path(adapter, Scope::User).unwrap();
    write(&path, "[mcp_servers.a]\npassword = your_secret_here\n");
    let error = app
        .read_client_config("codex", Scope::User)
        .unwrap_err()
        .to_string();
    assert!(error.contains("解析失败"));
    assert!(!error.contains("your_secret_here"));
    fs::write(&path, [0xff, 0xfe]).unwrap();
    assert!(app
        .read_client_config("codex", Scope::User)
        .unwrap_err()
        .to_string()
        .contains("编码"));
}

#[test]
fn jsonc_unicode_comments_trailing_commas_and_bom_are_supported() {
    let app = fixture();
    let path = app
        .target_path(adapters::by_id("opencode").unwrap(), Scope::User)
        .unwrap();
    write(&path, "\u{feff}{ // 注释\n\"mcp\":{\"servers\":{\"中文😀\":{\"command\":[\"python\",\"@scope/mcp\"],},},},}");
    let config = app.read_client_config("opencode", Scope::User).unwrap();
    assert_eq!(config.entries[0].name, "中文😀");
    assert_eq!(
        config.entries[0].config["command"],
        json!(["python", "@scope/mcp"])
    );
}

#[test]
fn fallback_file_uses_its_actual_format_and_priority() {
    let app = fixture();
    let templates = adapters::paths::candidates("grok", Scope::User, app.platform);
    let paths = app.env.expand_all(&templates, app.platform);
    write(
        &paths[1],
        r#"{"mcp_servers":{"from-json":{"command":"python"}}}"#,
    );
    let result = app.read_client_config("grok", Scope::User).unwrap();
    assert_eq!(result.format, Format::Json);
    assert_eq!(result.entries[0].name, "from-json");
    write(&paths[0], "[mcp_servers.from-toml]\ncommand = 'python'\n");
    let result = app.read_client_config("grok", Scope::User).unwrap();
    assert_eq!(result.format, Format::Toml);
    assert_eq!(result.entries[0].name, "from-toml");
    write(&paths[0], "broken your_secret_here");
    assert!(
        app.read_client_config("grok", Scope::User).is_err(),
        "优先文件损坏不能默默改读其他文件"
    );
}

#[test]
fn scopes_are_isolated_and_unknown_or_unsupported_requests_fail() {
    let app = fixture();
    let adapter = adapters::by_id("codex").unwrap();
    for (scope, name) in [(Scope::User, "user-only"), (Scope::Project, "project-only")] {
        let path = app.target_path(adapter, scope).unwrap();
        write(
            &path,
            &format!("[mcp_servers.{name}]\ncommand = 'python'\n"),
        );
        let result = app.read_client_config("codex", scope).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].name, name);
    }
    assert!(app.read_client_config("not-an-agent", Scope::User).is_err());
    assert!(app.read_client_config("codex", Scope::Local).is_err());
    // 已提供 project_root 时 Claude Code 本地级已打通（指向 .claude.json#projects.<proj>.mcpServers）
    let cc_local = app.read_client_config("claude-code", Scope::Local).unwrap();
    assert!(!cc_local.existed);
    let no_project = App::with_home(
        app.engine.home().to_path_buf(),
        HostEnv {
            project_root: None,
            ..app.env.clone()
        },
        app.platform,
    );
    assert!(no_project
        .read_client_config("claude-code", Scope::Local)
        .is_err());
    assert!(no_project
        .read_client_config("codex", Scope::Project)
        .is_err());
}

#[test]
fn credentials_in_any_value_or_argument_never_leave_backend() {
    let app = fixture();
    let path = app
        .target_path(adapters::by_id("codex").unwrap(), Scope::User)
        .unwrap();
    write(&path, "mcp_servers = { private = { command = 'python', args = ['--password', 'your_secret_here'], url = 'https://your_user:your_password_here@example.invalid/mcp?key=your_api_key_here', env = { PASSWORD = 'your_password_here', PIN = 1234 }, headers = { Authorization = 'Bearer your_token_here' }, custom = 'ordinary text', nested = [{ secret = 'your_secret_here' }] } }\n");
    let result = app.read_client_config("codex", Scope::User).unwrap();
    let shown = result.entries[0].config.to_string();
    for hidden in [
        "your_secret_here",
        "your_password_here",
        "your_api_key_here",
        "your_token_here",
        "1234",
        "example.invalid",
    ] {
        assert!(!shown.contains(hidden), "敏感值不能出现在 DTO 中");
    }
    assert_eq!(result.entries[0].config["command"], "python");
    assert_eq!(result.entries[0].config["custom"], "ordinary text");
}
