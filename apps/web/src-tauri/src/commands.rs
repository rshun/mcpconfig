//! IPC 命令：**薄适配层**，只做参数转换与调用 `mcp-app`。
//!
//! 这里不写业务逻辑；换宿主（Electron / Web）只需重写这一层。

use crate::dto::{
    AppInfoDto, BackupDto, CatalogDto, ClientDto, ExistingConfigDto, FileDto, MatrixDto, PlanDto,
    ReceiptDto, SkippedDto,
};
use mcp_app::settings::GuiSettings;
use mcp_app::{App, PlanRequest};
use mcp_core::model::Scope;
use mcp_core::plan::MergeMode;
use std::collections::BTreeMap;

fn app() -> App {
    App::from_process(None)
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
pub fn create_catalog(path: String) -> Result<Vec<String>, String> {
    context(Scope::User, None)?
        .create_catalog(std::path::Path::new(&path))
        .map_err(err)
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

/// 检测本机客户端（只读）。
#[tauri::command]
pub fn detect(scope: String, project_root: Option<String>) -> Result<Vec<ClientDto>, String> {
    let scope = parse_scope(&scope)?;
    let a = context(scope, project_root)?;
    Ok(mcp_app::detect_all(&a.env, a.platform, scope)
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

/// 读取并校验清单。
#[tauri::command]
pub fn load_catalog(path: String) -> Result<CatalogDto, String> {
    let a = app();
    let (cat, issues) = a.load_catalog(std::path::Path::new(&path)).map_err(err)?;
    let value = serde_json::to_value(&cat).map_err(err)?;
    Ok(CatalogDto {
        entry_count: cat.entries.len(),
        catalog: value,
        issues,
    })
}

/// 保存清单（写回 YAML）。
#[tauri::command]
pub fn save_catalog(path: String, catalog: serde_json::Value) -> Result<Vec<String>, String> {
    let cat: mcp_core::Catalog =
        serde_json::from_value(catalog).map_err(|_| "清单字段结构无效，请检查输入".to_string())?;
    context(Scope::User, None)?
        .save_catalog(std::path::Path::new(&path), &cat)
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
) -> Result<PlanDto, String> {
    let a = context(parse_scope(&scope)?, project_root)?;
    let mut req = PlanRequest::new(catalog_path);
    req.scope = parse_scope(&scope)?;
    req.mode = parse_mode(&mode)?;
    req.allow_comment_strip = allow_comment_strip;
    req.selection = Some(selection);

    let out = a.plan(&req).map_err(err)?;
    let writable = out.writable();
    Ok(PlanDto {
        entry_count: out.entry_count,
        issues: out.catalog_issues,
        missing_vars: out.missing_vars,
        skipped: out
            .skipped
            .into_iter()
            .map(|(client_id, reason)| SkippedDto { client_id, reason })
            .collect(),
        files: out.plans.into_iter().map(FileDto::from).collect(),
        writable,
    })
}

/// 应用：重新计算并写入（不做隐式确认，前端负责让用户点确认）。
#[tauri::command]
pub fn apply(
    catalog_path: String,
    selection: BTreeMap<String, Vec<String>>,
    scope: String,
    mode: String,
    allow_comment_strip: bool,
    dry_run: bool,
    project_root: Option<String>,
) -> Result<ReceiptDto, String> {
    let a = context(parse_scope(&scope)?, project_root)?;
    let mut req = PlanRequest::new(catalog_path);
    req.scope = parse_scope(&scope)?;
    req.mode = parse_mode(&mode)?;
    req.allow_comment_strip = allow_comment_strip;
    req.selection = Some(selection);

    let out = a.plan(&req).map_err(err)?;
    let receipt = a.apply(&out.plans, dry_run).map_err(err)?;
    Ok(ReceiptDto::from(receipt))
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
