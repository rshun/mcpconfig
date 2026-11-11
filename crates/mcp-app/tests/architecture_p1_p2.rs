use mcp_app::{
    catalog_session::CatalogSessions,
    importing::{ImportAction, ImportChoice, ImportSessions},
    App, CellState, HostEnv, PlanRequest,
};
use mcp_core::{plan::MergeMode, Platform, Scope};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

fn sandbox(tag: &str) -> (PathBuf, App) {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let n = SERIAL.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "mcphub-p1p2-{tag}-{}-{}-{n}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let proj = dir.join("work/my.app.v2");
    fs::create_dir_all(&proj).unwrap();
    let app = App::with_home(
        dir.join(".mcp-hub"),
        HostEnv {
            home: dir.clone(),
            appdata: Some(dir.join("AppData/Roaming")),
            project_root: Some(proj),
        },
        Platform::Windows,
    );
    (dir, app)
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

#[test]
fn p1_1_variable_and_secret_pipeline_respects_scope_dialect_and_blocks_unresolved_literal_only() {
    struct EnvGuard(String);
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(&self.0);
        }
    }

    let (dir, app) = sandbox("p1-1");
    let var_name = format!("MCPHUB_P1_SECRET_{}", std::process::id());
    let _guard = EnvGuard(var_name.clone());
    let secret_val = "your_secret_here_999";
    std::env::set_var(&var_name, secret_val);

    let cat_path = dir.join("catalog.yaml");
    write_file(
        &cat_path,
        &format!(
            r#"version: 1
entries:
  - name: env_srv
    transport: stdio
    command: "${{PROJECT_ROOT}}/bin/runner"
    env:
      API_KEY: "${{ENV:{var_name}}}"
  - name: auth_srv
    transport: http
    url: https://example.invalid/mcp
    auth:
      type: bearer
      token_env: {var_name}
"#
        ),
    );

    // 1. Scope::Project：绝不将宿主进程环境变量明文内联进受版本控制的项目文件，并翻译方言；
    //    LiteralOnly 客户端（vscode / trae）在 Project 作用域下因无法安全内联 Bearer 而阻断写入。
    let mut proj_req = PlanRequest::new(&cat_path);
    proj_req.scope = Scope::Project;
    proj_req.clients = Some(vec![
        "claude-code".into(),
        "opencode".into(),
        "grok".into(),
        "vscode".into(),
        "trae".into(),
    ]);
    let proj_out = app.plan(&proj_req).unwrap();

    for literal_id in ["vscode", "trae"] {
        let p = proj_out
            .plans
            .iter()
            .find(|p| p.client_id == literal_id)
            .unwrap();
        assert!(
            p.blocked.is_some(),
            "Project 作用域下 LiteralOnly 客户端 ({literal_id}) 必须阻断未解析内联凭据"
        );
    }

    let receipt = app.apply(&proj_out.plans, false).unwrap();
    assert_eq!(receipt.failed(), 0);

    let cc_proj =
        fs::read_to_string(app.env.project_root.as_ref().unwrap().join(".mcp.json")).unwrap();
    assert!(
        !cc_proj.contains(secret_val),
        "项目级配置绝不能泄露宿主进程环境变量明文"
    );
    assert!(!cc_proj.contains("<需内联:"), "绝不能落盘哨兵字符串");
    assert!(cc_proj.contains(&format!("${{{var_name}}}")));
    assert!(cc_proj.contains("my.app.v2/bin/runner") || cc_proj.contains("my.app.v2\\bin\\runner"));

    let oc_proj =
        fs::read_to_string(app.env.project_root.as_ref().unwrap().join("opencode.json")).unwrap();
    assert!(!oc_proj.contains(secret_val));
    assert!(oc_proj.contains(&format!("{{env:{var_name}}}")));

    let grok_proj =
        fs::read_to_string(app.env.project_root.as_ref().unwrap().join(".grok/config.toml"))
            .unwrap();
    assert!(!grok_proj.contains(secret_val));
    assert!(grok_proj.contains(&format!("${{{var_name}}}")));

    assert!(
        !app.env
            .project_root
            .as_ref()
            .unwrap()
            .join(".trae/mcp.json")
            .exists(),
        "被阻断的 LiteralOnly 客户端不能落盘"
    );

    // 2. Scope::User：LiteralOnly 客户端（cursor）在环境变量存在时安全解析，缺失时阻断写入（绝不写 <需内联:...>）；
    //    Windsurf（${env:VAR} 方言）在环境变量缺失时翻译为 ${env:VAR}
    let mut user_req = PlanRequest::new(&cat_path);
    user_req.scope = Scope::User;
    user_req.clients = Some(vec!["cursor".into()]);
    let user_ok = app.plan(&user_req).unwrap();
    assert_eq!(user_ok.writable(), 1);
    assert!(
        !user_ok.plans[0].diff_text().contains(secret_val),
        "Diff 展示必须脱敏"
    );
    app.apply(&user_ok.plans, false).unwrap();
    let cursor_path = dir.join(".cursor/mcp.json");
    let cursor_text = fs::read_to_string(&cursor_path).unwrap();
    assert!(cursor_text.contains(&format!("Bearer {secret_val}")));
    assert!(!cursor_text.contains("<需内联:"));

    // 移除环境变量后再次规划：cursor 阻断写入，windsurf 写入原生 ${env:VAR}
    std::env::remove_var(&var_name);
    let user_blocked = app.plan(&user_req).unwrap();
    assert_eq!(user_blocked.writable(), 0);
    assert!(user_blocked.plans[0].blocked.is_some());
    assert_eq!(app.apply(&user_blocked.plans, false).unwrap().written(), 0);

    let mut windsurf_req = PlanRequest::new(&cat_path);
    windsurf_req.scope = Scope::User;
    windsurf_req.clients = Some(vec!["windsurf".into()]);
    let ws_out = app.plan(&windsurf_req).unwrap();
    app.apply(&ws_out.plans, false).unwrap();
    let ws_text = fs::read_to_string(dir.join(".codeium/windsurf/mcp_config.json")).unwrap();
    assert!(ws_text.contains(&format!("${{env:{var_name}}}")));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn p1_2_three_way_merge_deletes_removed_keys_preserves_user_fields_fixes_matrix_and_roundtrips_extensions()
{
    let (dir, app) = sandbox("p1-2");
    let claude_path = dir.join(".claude.json");
    write_file(
        &claude_path,
        r#"{
  "mcpServers": {
    "alpha": {
      "command": "npx",
      "alwaysAllow": ["user_manual_tool"],
      "env": {
        "USER_CUSTOM": "keep_me"
      }
    }
  }
}"#,
    );

    let cat_path = dir.join("catalog.yaml");
    write_file(
        &cat_path,
        r#"version: 1
entries:
  - name: alpha
    transport: stdio
    command: npx
    env:
      MANAGED_ONE: "1"
      MANAGED_TWO: "your_secret_here_two"
    extensions:
      claude-code:
        autoApprove: ["hub_approved_tool"]
"#,
    );

    let mut req = PlanRequest::new(&cat_path);
    req.clients = Some(vec!["claude-code".into()]);
    req.mode = MergeMode::Merge;

    let plan1 = app.plan(&req).unwrap();
    assert_eq!(app.apply(&plan1.plans, false).unwrap().written(), 1);

    let state_raw = fs::read_to_string(app.engine.managed_state_path()).unwrap();
    assert!(
        !state_raw.contains("your_secret_here_two") && !state_raw.contains("hub_approved_tool"),
        "managed-state.json 只能保存键骨架，严禁包含任何标量值或密钥"
    );

    let v1: Value = serde_json::from_str(&fs::read_to_string(&claude_path).unwrap()).unwrap();
    assert_eq!(v1["mcpServers"]["alpha"]["env"]["MANAGED_ONE"], "1");
    assert_eq!(
        v1["mcpServers"]["alpha"]["env"]["MANAGED_TWO"],
        "your_secret_here_two"
    );
    assert_eq!(v1["mcpServers"]["alpha"]["env"]["USER_CUSTOM"], "keep_me");
    assert_eq!(
        v1["mcpServers"]["alpha"]["alwaysAllow"],
        json!(["user_manual_tool"])
    );
    assert_eq!(
        v1["mcpServers"]["alpha"]["autoApprove"],
        json!(["hub_approved_tool"])
    );

    // 矩阵此时应报告一致 Ok
    let m1 = app.matrix(&cat_path, Scope::User).unwrap();
    let c1 = m1.rows[0]
        .cells
        .iter()
        .find(|c| c.client_id == "claude-code")
        .unwrap();
    assert_eq!(c1.state, CellState::Ok);

    // 从清单中删除 MANAGED_TWO 与 autoApprove：矩阵应立即识别出漂移 Diff，应用后精准删除托管键并保留用户键
    write_file(
        &cat_path,
        r#"version: 1
entries:
  - name: alpha
    transport: stdio
    command: npx
    env:
      MANAGED_ONE: "1"
"#,
    );

    let m2 = app.matrix(&cat_path, Scope::User).unwrap();
    let c2 = m2.rows[0]
        .cells
        .iter()
        .find(|c| c.client_id == "claude-code")
        .unwrap();
    assert_eq!(
        c2.state,
        CellState::Diff,
        "删除清单中已托管键后，矩阵必须立即报告 Diff 而非误报 Ok"
    );

    let plan2 = app.plan(&req).unwrap();
    assert_eq!(plan2.writable(), 1);
    assert_eq!(app.apply(&plan2.plans, false).unwrap().written(), 1);

    let v2: Value = serde_json::from_str(&fs::read_to_string(&claude_path).unwrap()).unwrap();
    assert_eq!(v2["mcpServers"]["alpha"]["env"]["MANAGED_ONE"], "1");
    assert!(
        v2["mcpServers"]["alpha"]["env"].get("MANAGED_TWO").is_none(),
        "已从清单移除的托管键必须被清理"
    );
    assert!(
        v2["mcpServers"]["alpha"].get("autoApprove").is_none(),
        "已移除的托管扩展键必须被清理"
    );
    assert_eq!(
        v2["mcpServers"]["alpha"]["env"]["USER_CUSTOM"],
        "keep_me",
        "用户手写环境变量必须保留"
    );
    assert_eq!(
        v2["mcpServers"]["alpha"]["alwaysAllow"],
        json!(["user_manual_tool"]),
        "用户手写扩展字段必须保留"
    );

    let m3 = app.matrix(&cat_path, Scope::User).unwrap();
    assert_eq!(
        m3.rows[0]
            .cells
            .iter()
            .find(|c| c.client_id == "claude-code")
            .unwrap()
            .state,
        CellState::Ok
    );

    // 验证 importing 支持已知安全扩展字段（alwaysAllow / disabledTools）无损导入
    let imported_cat = dir.join("imported-catalog.yaml");
    let mut imports = ImportSessions::default();
    let scan = imports
        .scan(&app, &imported_cat, "claude-code", Scope::User)
        .unwrap();
    let alpha_item = scan.items.iter().find(|i| i.name == "alpha").unwrap();
    assert!(alpha_item.blocked.is_none());
    let prev = imports
        .preview(
            &app,
            &scan.scan_id,
            &[ImportChoice {
                source_name: "alpha".into(),
                action: ImportAction::Add,
                target_name: "alpha".into(),
                bindings: BTreeMap::new(),
                copy_fields: BTreeSet::new(),
            }],
        )
        .unwrap();
    imports.commit(&app, &prev.preview_id).unwrap();
    let (loaded_cat, _) = app.load_catalog(&imported_cat).unwrap();
    assert_eq!(
        loaded_cat.entries[0].extensions["claude-code"]["alwaysAllow"],
        json!(["user_manual_tool"])
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn p1_3_catalog_sessions_redact_on_load_rehydrate_across_rename_and_enforce_cas() {
    let (dir, app) = sandbox("p1-3");
    let cat_path = dir.join("catalog.yaml");
    write_file(
        &cat_path,
        r#"version: 1
entries:
  - name: secret-srv
    transport: http
    url: https://example.invalid/mcp
    auth:
      type: header
      header: X-Api-Key
      value: your_header_secret_here
    env:
      API_TOKEN: your_env_secret_here
"#,
    );

    let mut sessions = CatalogSessions::default();
    let loaded = sessions.load(&app, &cat_path).unwrap();
    let ipc_json = serde_json::to_string(&loaded.catalog).unwrap();
    assert!(!ipc_json.contains("your_header_secret_here"));
    assert!(!ipc_json.contains("your_env_secret_here"));
    assert!(ipc_json.contains("[已隐藏]"));
    assert_eq!(
        loaded.catalog["entries"][0]["auth"]["header"],
        "X-Api-Key",
        "安全元数据（请求头名称）应保留供前端展示"
    );

    // 重命名条目并保持 [已隐藏] 不变，保存后原始凭据必须完整回填
    let mut edited = loaded.catalog.clone();
    edited["entries"][0]["name"] = json!("renamed-srv");
    edited["entries"][0]["description"] = json!("已更新描述");
    let renames = BTreeMap::from([("renamed-srv".to_string(), "secret-srv".to_string())]);
    let save_out = sessions
        .save(
            &app,
            &cat_path,
            edited.clone(),
            Some(&loaded.revision_id),
            Some(&renames),
        )
        .unwrap();

    let (disk_cat, _) = app.load_catalog(&cat_path).unwrap();
    assert_eq!(disk_cat.entries[0].name, "renamed-srv");
    assert_eq!(
        disk_cat.entries[0].env.get("API_TOKEN").map(String::as_str),
        Some("your_env_secret_here")
    );
    match &disk_cat.entries[0].auth {
        Some(mcp_core::Auth::Header {
            header,
            value: mcp_core::SecretSource::Literal(v),
            ..
        }) => {
            assert_eq!(header, "X-Api-Key");
            assert_eq!(v, "your_header_secret_here");
        }
        other => panic!("unexpected auth after rehydration: {other:?}"),
    }

    // 旧 revision_id 必须被拒绝
    assert!(sessions
        .save(&app, &cat_path, edited, Some(&loaded.revision_id), None)
        .is_err());

    // 外部并发修改磁盘文件后，即使持有最新会话修订号也必须触发 CAS 拒绝覆盖
    let reloaded = sessions.load(&app, &cat_path).unwrap();
    assert_eq!(reloaded.revision_id, save_out.revision_id.replace(
        &save_out.revision_id,
        &reloaded.revision_id
    ));
    write_file(
        &cat_path,
        "version: 1\nentries:\n  - { name: external-edit, transport: stdio, command: node }\n",
    );
    let cas_err = sessions
        .save(
            &app,
            &cat_path,
            reloaded.catalog,
            Some(&reloaded.revision_id),
            None,
        )
        .unwrap_err()
        .to_string();
    assert!(cas_err.contains("外部修改") || cas_err.contains("拒绝"));
    let (still_external, _) = app.load_catalog(&cat_path).unwrap();
    assert_eq!(still_external.entries[0].name, "external-edit");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn p2_4_custom_header_auth_and_platform_overrides_for_auth_timeout_and_enabled() {
    let (dir, mut app) = sandbox("p2-4");
    let cat_path = dir.join("catalog.yaml");
    write_file(
        &cat_path,
        r#"version: 1
entries:
  - name: gateway
    transport: http
    url: https://example.invalid/mcp
    timeout_seconds: 30
    enabled: true
    auth:
      type: header
      header: X-Gateway-Key
      prefix: "Key "
      value_env: GW_KEY
    overrides:
      windows:
        timeout_seconds: 120
        enabled: false
        auth:
          type: header
          header: X-Win-Key
          value_env: WIN_GW_KEY
  - name: codex-worker
    transport: stdio
    command: python
    timeout_seconds: 20
    enabled: true
    overrides:
      windows:
        timeout_seconds: 120
        enabled: false
"#,
    );

    let mut req = PlanRequest::new(&cat_path);
    req.scope = Scope::Project;
    req.selection = Some(BTreeMap::from([
        ("claude-code".into(), vec!["gateway".into()]),
        ("codex".into(), vec!["codex-worker".into()]),
    ]));

    app.platform = Platform::Windows;
    let win_plan = app.plan(&req).unwrap();
    app.apply(&win_plan.plans, false).unwrap();
    let cc_win: Value = serde_json::from_str(
        &fs::read_to_string(app.env.project_root.as_ref().unwrap().join(".mcp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        cc_win["mcpServers"]["gateway"]["headers"]["X-Win-Key"],
        "${WIN_GW_KEY}"
    );
    let codex_win = fs::read_to_string(
        app.env
            .project_root
            .as_ref()
            .unwrap()
            .join(".codex/config.toml"),
    )
    .unwrap();
    assert!(codex_win.contains("startup_timeout_sec = 120"));
    assert!(codex_win.contains("enabled = false"));

    app.platform = Platform::Linux;
    let linux_plan = app.plan(&req).unwrap();
    app.apply(&linux_plan.plans, false).unwrap();
    let cc_linux: Value = serde_json::from_str(
        &fs::read_to_string(app.env.project_root.as_ref().unwrap().join(".mcp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        cc_linux["mcpServers"]["gateway"]["headers"]["X-Gateway-Key"],
        "Key ${GW_KEY}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn p2_5_claude_code_local_scope_writes_nested_project_container_and_preserves_siblings() {
    let (dir, app) = sandbox("p2-5");
    let claude_json = dir.join(".claude.json");
    write_file(
        &claude_json,
        r#"{
    // 顶层偏好与全局 MCP
    "theme": "dark",
    "mcpServers": {
        "global-only": { "command": "npx" }
    },
    "projects": {
        "C:/other/repo": {
            "mcpServers": {
                "other-local": { "command": "python" }
            }
        }
    }
}"#,
    );

    let cat_path = dir.join("catalog.yaml");
    write_file(
        &cat_path,
        r#"version: 1
entries:
  - name: local-db
    transport: stdio
    command: uvx
    args: ["mcp-sqlite"]
    scope: local
"#,
    );

    let mut req = PlanRequest::new(&cat_path);
    req.scope = Scope::Local;
    req.clients = Some(vec!["claude-code".into()]);
    let out = app.plan(&req).unwrap();
    assert_eq!(out.writable(), 1);
    assert_eq!(app.apply(&out.plans, false).unwrap().written(), 1);

    let raw_after = fs::read_to_string(&claude_json).unwrap();
    assert!(
        raw_after.contains("// 顶层偏好与全局 MCP"),
        "JSONC 区间补丁应保留外层注释与 4 空格缩进"
    );

    let inspected_local = app.read_client_config("claude-code", Scope::Local).unwrap();
    assert_eq!(inspected_local.entries.len(), 1);
    assert_eq!(inspected_local.entries[0].name, "local-db");

    let inspected_user = app.read_client_config("claude-code", Scope::User).unwrap();
    assert_eq!(inspected_user.entries.len(), 1);
    assert_eq!(inspected_user.entries[0].name, "global-only");

    let detected_local = app.detect_one("claude-code", Scope::Local).unwrap().unwrap();
    assert_eq!(detected_local.configured_entries, vec!["local-db"]);

    let matrix_local = app.matrix(&cat_path, Scope::Local).unwrap();
    let cc_cell = matrix_local.rows[0]
        .cells
        .iter()
        .find(|c| c.client_id == "claude-code")
        .unwrap();
    assert_eq!(cc_cell.state, CellState::Ok);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn p2_6_declarative_custom_adapters_yaml_supports_end_to_end_lifecycle_and_rejects_unsafe_specs() {
    let (dir, app) = sandbox("p2-6");
    write_file(
        &app.custom_adapters_path(),
        r#"version: 1
adapters:
  - id: continue-yaml
    display_name: Continue Dev
    kind: ide
    format: yaml
    root_key: [mcpServers]
    secret_style:
      kind: env_expansion
      prefix: "${"
      suffix: "}"
    paths:
      user:
        all: ["~/.continue/mcp.yaml"]
      project:
        all: ["<proj>/.continue/mcp.yaml"]
"#,
    );

    let detected = app.detect_all(Scope::User).unwrap();
    assert_eq!(detected.len(), 15);
    assert!(detected.iter().any(|c| c.id == "continue-yaml"));

    let cat_path = dir.join("catalog.yaml");
    write_file(
        &cat_path,
        r#"version: 1
entries:
  - name: yaml-srv
    transport: stdio
    command: npx
    args: ["-y", "srv"]
    env:
      API_KEY: your_yaml_secret_here
"#,
    );

    let mut req = PlanRequest::new(&cat_path);
    req.clients = Some(vec!["continue-yaml".into()]);
    let out = app.plan(&req).unwrap();
    assert_eq!(out.writable(), 1);
    assert_eq!(app.apply(&out.plans, false).unwrap().written(), 1);

    let yaml_file = dir.join(".continue/mcp.yaml");
    assert!(yaml_file.exists());
    let inspected = app
        .read_client_config("continue-yaml", Scope::User)
        .unwrap();
    assert_eq!(inspected.entries.len(), 1);
    assert_eq!(inspected.entries[0].name, "yaml-srv");
    assert_eq!(inspected.entries[0].config["env"]["API_KEY"], "[已隐藏]");

    let matrix = app.matrix(&cat_path, Scope::User).unwrap();
    assert!(matrix
        .client_details
        .iter()
        .any(|(id, name, _)| id == "continue-yaml" && name == "Continue Dev"));
    assert_eq!(
        matrix.rows[0]
            .cells
            .iter()
            .find(|c| c.client_id == "continue-yaml")
            .unwrap()
            .state,
        CellState::Ok
    );

    // 路径穿越或覆盖内置客户端 ID 必须被拒绝
    write_file(
        &app.custom_adapters_path(),
        r#"version: 1
adapters:
  - id: bad-traversal
    display_name: Bad
    kind: cli
    format: json
    root_key: [mcpServers]
    paths:
      user:
        all: ["~/../escape.json"]
"#,
    );
    assert!(app.detect_all(Scope::User).is_err());

    let _ = fs::remove_dir_all(&dir);
}
