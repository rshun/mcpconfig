//! IPC 命令：**薄适配层**，只做参数转换与调用 `mcp-app`。
//!
//! 这里不写业务逻辑；换宿主（Electron / Web）只需重写这一层。

use crate::dto::{
    AppInfoDto, BackupDto, CatalogDto, ClientDto, FileDto, MatrixDto, PlanDto, ReceiptDto,
    SkippedDto,
};
use mcp_app::{App, PlanRequest};
use mcp_core::model::Scope;
use mcp_core::plan::MergeMode;

fn app() -> App {
    App::from_process(None)
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
pub fn detect(scope: String) -> Result<Vec<ClientDto>, String> {
    let scope = parse_scope(&scope)?;
    let a = app();
    Ok(mcp_app::detect_all(&a.env, a.platform, scope)
        .into_iter()
        .map(ClientDto::from)
        .collect())
}

/// 矩阵视图（只读）。
#[tauri::command]
pub fn matrix(catalog_path: String, scope: String) -> Result<MatrixDto, String> {
    let scope = parse_scope(&scope)?;
    let a = app();
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
    let cat: mcp_core::Catalog = serde_json::from_value(catalog).map_err(err)?;

    let issues = mcp_core::catalog::validate(&cat);
    if let Some(first) = issues.iter().find(|i| i.is_error()) {
        return Err(format!("清单校验失败：{first}"));
    }

    let yaml = mcp_core::catalog::to_yaml(&cat).map_err(err)?;

    // 写前备份现有清单（复用引擎的备份目录约定）
    let target = std::path::Path::new(&path);
    if target.exists() {
        let a = app();
        let backup_dir = a.engine.backups_dir().join("catalog");
        let _ = std::fs::create_dir_all(&backup_dir);
        let stamp = mcp_io::time::UtcTime::now().stamp();
        let _ = std::fs::copy(target, backup_dir.join(format!("{stamp}.yaml")));
    }

    std::fs::write(target, yaml).map_err(err)?;
    Ok(issues.iter().map(|i| i.to_string()).collect())
}

/// 预演：计算逐文件变化（不写盘）。
#[tauri::command]
pub fn plan(
    catalog_path: String,
    clients: Option<Vec<String>>,
    scope: String,
    mode: String,
    allow_comment_strip: bool,
) -> Result<PlanDto, String> {
    let a = app();
    let mut req = PlanRequest::new(catalog_path);
    req.scope = parse_scope(&scope)?;
    req.mode = parse_mode(&mode)?;
    req.allow_comment_strip = allow_comment_strip;
    req.clients = clients;

    let out = a.plan(&req).map_err(err)?;
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
        writable: out.writable(),
    })
}

/// 应用：重新计算并写入（不做隐式确认，前端负责让用户点确认）。
#[tauri::command]
pub fn apply(
    catalog_path: String,
    clients: Option<Vec<String>>,
    scope: String,
    mode: String,
    allow_comment_strip: bool,
    dry_run: bool,
) -> Result<ReceiptDto, String> {
    let a = app();
    let mut req = PlanRequest::new(catalog_path);
    req.scope = parse_scope(&scope)?;
    req.mode = parse_mode(&mode)?;
    req.allow_comment_strip = allow_comment_strip;
    req.clients = clients;

    let out = a.plan(&req).map_err(err)?;
    let receipt = a.apply(&out.plans, dry_run).map_err(err)?;
    Ok(ReceiptDto::from(receipt))
}

/// 备份列表。
#[tauri::command]
pub fn backups() -> Result<Vec<BackupDto>, String> {
    let a = app();
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
    let a = app();
    a.restore(&id).map_err(err)
}
