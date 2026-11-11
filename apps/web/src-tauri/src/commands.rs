//! IPC 命令：**薄适配层**，只做参数转换与调用 `mcp-app`。
//!
//! 这里不写业务逻辑；换宿主（Electron / Web）只需重写这一层。

use crate::dto::{
    AppInfoDto, BackupDto, CatalogDto, ClientDto, ExistingConfigDto, MatrixDto, PlanDto, ReceiptDto,
};
use mcp_app::catalog_session::CatalogSessions;
use mcp_app::distribution::DistributionSessions;
use mcp_app::importing::{ImportChoice, ImportPreview, ImportResult, ImportScan, ImportSessions};
use mcp_app::settings::GuiSettings;
use mcp_app::{App, PlanRequest};
use mcp_core::model::Scope;
use mcp_core::plan::MergeMode;
use std::collections::BTreeMap;

fn app() -> App {
    App::from_process(None)
}

type Imports<'a> = tauri::State<'a, std::sync::Mutex<ImportSessions>>;
type Previews<'a> = tauri::State<'a, std::sync::Mutex<DistributionSessions>>;
type Catalogs<'a> = tauri::State<'a, std::sync::Mutex<CatalogSessions>>;

#[tauri::command]
pub fn scan_import(
    catalog_path: String,
    client_id: String,
    scope: String,
    project_root: Option<String>,
    imports: Imports<'_>,
) -> Result<ImportScan, String> {
    let scope = parse_scope(&scope)?;
    let app = context(scope, project_root)?;
    imports
        .lock()
        .map_err(|_| "导入状态不可用，请重启软件".to_string())?
        .scan(&app, std::path::Path::new(&catalog_path), &client_id, scope)
        .map_err(err)
}

#[tauri::command]
pub fn preview_import(
    scan_id: String,
    choices: Vec<ImportChoice>,
    scope: String,
    project_root: Option<String>,
    imports: Imports<'_>,
) -> Result<ImportPreview, String> {
    let app = context(parse_scope(&scope)?, project_root)?;
    imports
        .lock()
        .map_err(|_| "导入状态不可用，请重启软件".to_string())?
        .preview(&app, &scan_id, &choices)
        .map_err(err)
}

#[tauri::command]
pub fn commit_import(
    preview_id: String,
    scope: String,
    project_root: Option<String>,
    imports: Imports<'_>,
    catalogs: Catalogs<'_>,
) -> Result<ImportResult, String> {
    let app = context(parse_scope(&scope)?, project_root)?;
    let out = imports
        .lock()
        .map_err(|_| "导入状态不可用，请重启软件".to_string())?
        .commit(&app, &preview_id)
        .map_err(err)?;
    if let Ok(settings) = app.load_settings() {
        if let Ok(mut guard) = catalogs.lock() {
            guard.sync_path(std::path::Path::new(&settings.catalog_path));
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn cancel_import(scan_id: String, imports: Imports<'_>) -> Result<(), String> {
    imports
        .lock()
        .map_err(|_| "导入状态不可用，请重启软件".to_string())?
        .cancel(&scan_id);
    Ok(())
}

fn context(scope: Scope, project_root: Option<String>) -> Result<App, String> {
    app()
        .gui_context(scope, project_root.map(std::path::PathBuf::from))
        .map_err(err)
}

#[tauri::command]
pub fn load_settings() -> Result<GuiSettings, String> {
    app().load_settings().map_err(err)
}

#[tauri::command]
pub fn default_settings() -> GuiSettings {
    app().default_settings()
}

#[tauri::command]
pub fn save_settings(settings: GuiSettings) -> Result<Vec<String>, String> {
    app().save_settings(&settings).map_err(err)
}

#[tauri::command]
pub async fn choose_directory(initial: Option<String>) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::directory::choose(initial))
        .await
        .map_err(|_| "目录选择任务失败，请手动输入目录".to_string())?
}

#[tauri::command]
pub fn create_catalog(path: String, catalogs: Catalogs<'_>) -> Result<Vec<String>, String> {
    let target = std::path::Path::new(&path);
    let warnings = context(Scope::User, None)?
        .create_catalog(target)
        .map_err(err)?;
    if let Ok(mut guard) = catalogs.lock() {
        guard.sync_path(target);
    }
    Ok(warnings)
}

fn parse_scope(s: &str) -> Result<Scope, String> {
    match s {
        "user" | "global" => Ok(Scope::User),
        "project" => Ok(Scope::Project),
        "local" => Ok(Scope::Local),
        other => Err(format!("未知作用域：{other}")),
    }
}

fn parse_mode(s: &str) -> Result<MergeMode, String> {
    match s {
        "merge" => Ok(MergeMode::Merge),
        "replace" => Ok(MergeMode::Replace),
        other => Err(format!("未知覆盖方式：{other}")),
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 应用与目录信息。
#[tauri::command]
pub fn app_info() -> AppInfoDto {
    let a = app();
    AppInfoDto {
        version: mcp_app::version().to_string(),
        platform: a.platform.as_str().to_string(),
        home: a.env.home.display().to_string(),
        project_root: a
            .env
            .project_root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        data_dir: a.engine.home().display().to_string(),
    }
}

/// 检测本机客户端（只读，含 `adapters.yaml` 自定义客户端）。
#[tauri::command]
pub fn detect(scope: String, project_root: Option<String>) -> Result<Vec<ClientDto>, String> {
    let scope = parse_scope(&scope)?;
    let a = context(scope, project_root)?;
    Ok(a.detect_all(scope)
        .map_err(err)?
        .into_iter()
        .map(ClientDto::from)
        .collect())
}

/// 按预定义路径只读查看已有 MCP；敏感值已经在应用层隐藏。
#[tauri::command]
pub fn read_client_config(
    client_id: String,
    scope: String,
    project_root: Option<String>,
) -> Result<ExistingConfigDto, String> {
    let scope = parse_scope(&scope)?;
    context(scope, project_root)?
        .read_client_config(&client_id, scope)
        .map(ExistingConfigDto::from)
        .map_err(err)
}

/// 矩阵视图（只读）。
#[tauri::command]
pub fn matrix(
    catalog_path: String,
    scope: String,
    project_root: Option<String>,
) -> Result<MatrixDto, String> {
    let scope = parse_scope(&scope)?;
    let a = context(scope, project_root)?;
    a.matrix(std::path::Path::new(&catalog_path), scope)
        .map(MatrixDto::from)
        .map_err(err)
}

/// 读取并校验清单（敏感凭据在离开后端前隐藏为 `[已隐藏]`）。
#[tauri::command]
pub fn load_catalog(path: String, catalogs: Catalogs<'_>) -> Result<CatalogDto, String> {
    let a = app();
    let mut sessions = catalogs
        .lock()
        .map_err(|_| "配置库会话状态不可用，请重启软件".to_string())?;
    read_catalog(&mut sessions, &a, std::path::Path::new(&path))
}

fn read_catalog(
    sessions: &mut CatalogSessions,
    a: &App,
    path: &std::path::Path,
) -> Result<CatalogDto, String> {
    let loaded = sessions.load(a, path).map_err(err)?;
    Ok(CatalogDto {
        exists: loaded.exists,
        revision_id: Some(loaded.revision_id),
        catalog: loaded.catalog,
        issues: loaded.issues,
        entry_count: loaded.entry_count,
    })
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    #[test]
    fn absent_library_is_an_empty_state_corruption_is_error_and_secrets_are_redacted() {
        let root = std::env::temp_dir().join(format!(
            "mcphub-desktop-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let app = App::with_home(
            root.join("data"),
            mcp_app::HostEnv {
                home: root.clone(),
                appdata: None,
                project_root: None,
            },
            mcp_core::Platform::current(),
        );
        let mut sessions = CatalogSessions::default();
        let path = root.join("catalog.yaml");
        assert!(!read_catalog(&mut sessions, &app, &path).unwrap().exists);
        assert!(!path.exists());
        assert!(!app.engine.home().exists());
        std::fs::write(&path, "broken: your_secret_here").unwrap();
        let error = read_catalog(&mut sessions, &app, &path).err().unwrap();
        assert!(!error.contains("your_secret_here"));
        std::fs::write(
            &path,
            "version: 1\nentries:\n  - name: s\n    transport: stdio\n    command: python\n    env:\n      API_KEY: your_secret_here\n",
        )
        .unwrap();
        let dto = read_catalog(&mut sessions, &app, &path).unwrap();
        assert!(dto.exists);
        assert!(dto.revision_id.is_some());
        let serialized = serde_json::to_string(&dto).unwrap();
        assert!(!serialized.contains("your_secret_here"));
        assert!(serialized.contains("[已隐藏]"));
    }
}

/// 保存清单（CAS 校验修订号与磁盘并发状态，自动回填未改动的 `[已隐藏]` 凭据并写回 YAML/JSON）。
#[tauri::command]
pub fn save_catalog(
    path: String,
    catalog: serde_json::Value,
    revision_id: Option<String>,
    renames: Option<BTreeMap<String, String>>,
    catalogs: Catalogs<'_>,
) -> Result<Vec<String>, String> {
    let a = context(Scope::User, None)?;
    let mut sessions = catalogs
        .lock()
        .map_err(|_| "配置库会话状态不可用，请重启软件".to_string())?;
    sessions
        .save(
            &a,
            std::path::Path::new(&path),
            catalog,
            revision_id.as_deref(),
            renames.as_ref(),
        )
        .map(|out| out.warnings)
        .map_err(err)
}

/// 预演：计算逐文件变化（不写盘）。
#[tauri::command]
pub fn plan(
    catalog_path: String,
    selection: BTreeMap<String, Vec<String>>,
    scope: String,
    mode: String,
    allow_comment_strip: bool,
    project_root: Option<String>,
    previews: Previews<'_>,
) -> Result<PlanDto, String> {
    let mut previews = previews
        .lock()
        .map_err(|_| "分发预览状态不可用，请重启软件".to_string())?;
    previews.invalidate();
    let a = context(parse_scope(&scope)?, project_root)?;
    let mut req = PlanRequest::new(catalog_path);
    req.scope = parse_scope(&scope)?;
    req.mode = parse_mode(&mode)?;
    req.allow_comment_strip = allow_comment_strip;
    req.selection = Some(selection);

    let (preview_id, out) = previews.preview(&a, &req).map_err(err)?;
    Ok(PlanDto::from((preview_id, out)))
}

/// 确认：校验一次性预览，再写入后端保留的原始计划。不接受无预览 ID 的写入。
#[tauri::command]
pub fn apply(
    catalog_path: String,
    selection: BTreeMap<String, Vec<String>>,
    scope: String,
    mode: String,
    allow_comment_strip: bool,
    dry_run: bool,
    project_root: Option<String>,
    preview_id: String,
    previews: Previews<'_>,
) -> Result<ReceiptDto, String> {
    let mut previews = previews
        .lock()
        .map_err(|_| "分发预览状态不可用，请重启软件".to_string())?;
    let result = (|| {
        let a = context(parse_scope(&scope)?, project_root)?;
        let mut req = PlanRequest::new(catalog_path);
        req.scope = parse_scope(&scope)?;
        req.mode = parse_mode(&mode)?;
        req.allow_comment_strip = allow_comment_strip;
        req.selection = Some(selection);
        previews
            .commit(&a, &req, &preview_id, dry_run)
            .map(ReceiptDto::from)
            .map_err(err)
    })();
    // 包括上下文构造失败：确认失败后也必须重新预览。
    if result.is_err() {
        previews.cancel(&preview_id);
    }
    result
}

#[tauri::command]
pub fn cancel_plan(preview_id: String, previews: Previews<'_>) -> Result<(), String> {
    previews
        .lock()
        .map_err(|_| "分发预览状态不可用，请重启软件".to_string())?
        .cancel(&preview_id);
    Ok(())
}

/// 备份列表。
#[tauri::command]
pub fn backups() -> Result<Vec<BackupDto>, String> {
    let a = context(Scope::User, None)?;
    let list = a.backups().map_err(err)?;
    Ok(list
        .into_iter()
        .map(|b| BackupDto {
            id: b.id,
            created_at: b.created_at,
            files: b.files.len(),
            paths: b.files.into_iter().map(|f| f.path).collect(),
        })
        .collect())
}

/// 回滚到指定备份。
#[tauri::command]
pub fn restore(id: String) -> Result<usize, String> {
    let a = context(Scope::User, None)?;
    a.restore(&id).map_err(err)
}
