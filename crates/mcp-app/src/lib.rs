//! # mcp-app
//!
//! 应用服务层：**宿主无关**的用例编排。
//!
//! GUI（Tauri）与 CLI 都只调用这里，不直接碰 `mcp-io` / `mcp-core`。
//! 因此换前端不需要改动任何业务逻辑（见架构文档 §3.2）。

pub mod detect;
pub mod env;

use mcp_core::adapters::{self, Adapter};
use mcp_core::interpolate::VarCtx;
use mcp_core::model::{Catalog, McpEntry, Platform, Scope};
use mcp_core::plan::{plan_file, FilePlan, MergeMode};
use mcp_core::{catalog, resolve};
use mcp_io::{ApplyOptions, ApplyReceipt, BackupManifest, Engine};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use detect::{detect_all, detect_client, DetectState};
pub use env::HostEnv;

// ---------------------------------------------------------------- 作用域能力

/// v0 的应用层作用域限制。
///
/// Claude Code 的 `local` 需要写入 `.claude.json` 内部的
/// `projects.<路径>.mcpServers` 嵌套结构，当前文档模型不支持，故暂时禁用。
pub fn scope_block_reason(client_id: &str, scope: Scope) -> Option<String> {
    let adapter = adapters::by_id(client_id)?;
    if !adapter.supports_scope(scope) {
        return Some(format!("{} 不支持{}作用域", adapter.display_name(), scope));
    }
    if client_id == "claude-code" && scope == Scope::Local {
        return Some("v0 暂不支持 Claude Code 的本地级（需嵌套写入 projects.<路径>）".into());
    }
    None
}

// ---------------------------------------------------------------- 应用

pub struct App {
    pub engine: Engine,
    pub env: HostEnv,
    pub platform: Platform,
}

impl App {
    /// 使用默认位置（`~/.mcp-hub`）与当前进程环境。
    pub fn from_process(project_root: Option<PathBuf>) -> Self {
        Self {
            engine: Engine::with_default_home(),
            env: HostEnv::from_process(project_root),
            platform: Platform::current(),
        }
    }

    /// 自定义 home（测试用）。
    pub fn with_home(home: PathBuf, env: HostEnv, platform: Platform) -> Self {
        Self {
            engine: Engine::new(home),
            env,
            platform,
        }
    }

    /// 构建插值上下文。
    pub fn var_ctx(&self) -> VarCtx {
        VarCtx {
            home: Some(self.env.home.to_string_lossy().to_string()),
            project_root: self
                .env
                .project_root
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            vars: BTreeMap::new(),
            read_process_env: true,
        }
    }

    /// 读取并校验清单。
    pub fn load_catalog(&self, path: &Path) -> Result<(Catalog, Vec<String>), AppError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| AppError::Other(format!("读取清单 {} 失败：{e}", path.display())))?;
        let (cat, issues) = catalog::load(&text)?;
        Ok((cat, issues.iter().map(|i| i.to_string()).collect()))
    }

    /// 制定应用计划（不写盘）。
    pub fn plan(&self, req: &PlanRequest) -> Result<PlanOutcome, AppError> {
        let (cat, issues) = self.load_catalog(&req.catalog_path)?;
        let ctx = self.var_ctx();
        let (resolved, missing) = resolve::resolve_catalog(&cat, self.platform, &ctx);

        let entries: Vec<McpEntry> = resolve::entries_for_scope(&resolved, req.scope, req.scope);

        let ids: Vec<String> = match &req.clients {
            Some(list) => list.clone(),
            None => adapters::all().iter().map(|a| a.id().to_string()).collect(),
        };

        let mut plans = Vec::new();
        let mut skipped = Vec::new();

        for id in &ids {
            let Some(adapter) = adapters::by_id(id) else {
                return Err(AppError::UnknownClient(id.clone()));
            };

            if let Some(reason) = scope_block_reason(id, req.scope) {
                skipped.push((id.clone(), reason));
                continue;
            }

            let Some(path) = self.target_path(adapter, req.scope) else {
                skipped.push((id.clone(), "无法解析配置路径（缺少环境变量）".into()));
                continue;
            };

            let existing = self
                .engine
                .read_text(&path)
                .map_err(|e| AppError::Other(e.to_string()))?;

            let plan = plan_file(
                adapter,
                path.display().to_string(),
                req.scope,
                existing.as_deref(),
                &entries,
                req.mode,
                req.allow_comment_strip,
            )?;
            plans.push(plan);
        }

        Ok(PlanOutcome {
            catalog_issues: issues,
            missing_vars: missing.into_iter().map(|m| m.raw).collect(),
            plans,
            skipped,
            entry_count: entries.len(),
        })
    }

    /// 目标路径：优先已存在者，否则第一个候选。
    pub fn target_path(&self, adapter: &dyn Adapter, scope: Scope) -> Option<PathBuf> {
        let templates = mcp_core::adapters::paths::candidates(adapter.id(), scope, self.platform);
        let candidates = self.env.expand_all(&templates, self.platform);
        candidates
            .iter()
            .find(|p| p.exists())
            .cloned()
            .or_else(|| candidates.first().cloned())
    }

    /// 执行写入。
    pub fn apply(&self, plans: &[FilePlan], dry_run: bool) -> Result<ApplyReceipt, AppError> {
        Ok(self.engine.apply(plans, &ApplyOptions { dry_run })?)
    }

    pub fn backups(&self) -> Result<Vec<BackupManifest>, AppError> {
        Ok(self.engine.list_backups()?)
    }

    pub fn restore(&self, backup_id: &str) -> Result<usize, AppError> {
        Ok(self.engine.restore(backup_id)?)
    }
}

// ---------------------------------------------------------------- 矩阵视图

/// 单元格状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellState {
    /// 已配置且与清单一致
    Ok,
    /// 未配置
    Absent,
    /// 已配置但有差异
    Diff,
    /// 该客户端不支持此作用域
    Unsupported,
}

impl CellState {
    pub fn label(self) -> &'static str {
        match self {
            CellState::Ok => "ok",
            CellState::Absent => "absent",
            CellState::Diff => "diff",
            CellState::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MatrixCell {
    pub client_id: String,
    pub state: CellState,
    /// 不支持时的原因
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MatrixRow {
    pub name: String,
    pub transport: String,
    pub scope: Scope,
    pub cells: Vec<MatrixCell>,
}

#[derive(Debug, Clone)]
pub struct MatrixView {
    pub platform: Platform,
    pub scope: Scope,
    pub clients: Vec<String>,
    pub rows: Vec<MatrixRow>,
    pub issues: Vec<String>,
}

impl App {
    /// 生成矩阵视图：每格是「该条目在该客户端」的真实状态。
    ///
    /// 只读：不会写入任何文件。
    pub fn matrix(&self, catalog_path: &Path, scope: Scope) -> Result<MatrixView, AppError> {
        let (cat, issues) = self.load_catalog(catalog_path)?;
        let ctx = self.var_ctx();
        let (resolved, _missing) = resolve::resolve_catalog(&cat, self.platform, &ctx);
        let entries = resolve::entries_for_scope(&resolved, scope, scope);

        let clients: Vec<String> = adapters::all().iter().map(|a| a.id().to_string()).collect();
        let mut rows = Vec::with_capacity(entries.len());

        // 每个客户端的文档只解析一次
        let mut docs: Vec<Option<mcp_core::document::ConfigDoc>> = Vec::with_capacity(clients.len());
        let mut roots: Vec<Option<Vec<String>>> = Vec::with_capacity(clients.len());

        for id in &clients {
            let adapter = adapters::by_id(id).unwrap();
            if scope_block_reason(id, scope).is_some() {
                docs.push(None);
                roots.push(None);
                continue;
            }
            let Some(path) = self.target_path(adapter, scope) else {
                docs.push(None);
                roots.push(None);
                continue;
            };
            let text = self
                .engine
                .read_text(&path)
                .map_err(|e| AppError::Other(e.to_string()))?
                .unwrap_or_default();
            let doc = mcp_core::document::ConfigDoc::parse(adapter.spec().format, &text).ok();
            docs.push(doc);
            roots.push(Some(adapter.root_key(scope)));
        }

        for entry in &entries {
            let mut cells = Vec::with_capacity(clients.len());
            for (i, id) in clients.iter().enumerate() {
                let adapter = adapters::by_id(id).unwrap();
                if let Some(reason) = scope_block_reason(id, scope) {
                    cells.push(MatrixCell {
                        client_id: id.clone(),
                        state: CellState::Unsupported,
                        detail: Some(reason),
                    });
                    continue;
                }
                let (Some(doc), Some(root)) = (&docs[i], &roots[i]) else {
                    cells.push(MatrixCell {
                        client_id: id.clone(),
                        state: CellState::Unsupported,
                        detail: Some("无法解析配置路径".into()),
                    });
                    continue;
                };
                let encoded = adapter.encode_entry(entry)?;
                let state = match doc.get(root, &entry.name) {
                    None => CellState::Absent,
                    Some(existing) => {
                        let merged = mcp_core::document::merge_json(&existing, &encoded);
                        if merged == existing {
                            CellState::Ok
                        } else {
                            CellState::Diff
                        }
                    }
                };
                cells.push(MatrixCell { client_id: id.clone(), state, detail: None });
            }
            rows.push(MatrixRow {
                name: entry.name.clone(),
                transport: match entry.transport {
                    mcp_core::Transport::Stdio => "stdio".into(),
                    mcp_core::Transport::Http => "http".into(),
                    mcp_core::Transport::Sse => "sse".into(),
                },
                scope: resolve::effective_scope(entry, scope),
                cells,
            });
        }

        Ok(MatrixView {
            platform: self.platform,
            scope,
            clients,
            rows,
            issues: issues.iter().map(|i| i.to_string()).collect(),
        })
    }
}

// ---------------------------------------------------------------- 请求/响应

#[derive(Debug, Clone)]
pub struct PlanRequest {
    pub catalog_path: PathBuf,
    /// `None` 表示全部客户端
    pub clients: Option<Vec<String>>,
    pub scope: Scope,
    pub mode: MergeMode,
    /// JSONC 文件含注释时是否允许重写（会丢注释）
    pub allow_comment_strip: bool,
}

impl PlanRequest {
    pub fn new(catalog_path: impl Into<PathBuf>) -> Self {
        Self {
            catalog_path: catalog_path.into(),
            clients: None,
            scope: Scope::User,
            mode: MergeMode::Merge,
            allow_comment_strip: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanOutcome {
    pub plans: Vec<FilePlan>,
    pub catalog_issues: Vec<String>,
    pub missing_vars: Vec<String>,
    /// 因作用域/路径原因未纳入的客户端
    pub skipped: Vec<(String, String)>,
    /// 参与本作用域的条目数
    pub entry_count: usize,
}

impl PlanOutcome {
    pub fn writable(&self) -> usize {
        self.plans.iter().filter(|p| p.has_write()).count()
    }
    pub fn blocked(&self) -> usize {
        self.plans.iter().filter(|p| p.blocked.is_some()).count()
    }
    pub fn total_new(&self) -> usize {
        self.plans.iter().map(|p| p.changed_count().0).sum()
    }
    pub fn total_update(&self) -> usize {
        self.plans.iter().map(|p| p.changed_count().1).sum()
    }
}

// ---------------------------------------------------------------- 错误

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Core(#[from] mcp_core::Error),
    #[error("{0}")]
    Io(#[from] mcp_io::IoError),
    #[error("未知客户端：{0}")]
    UnknownClient(String),
    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------- 便捷函数

/// 版本号（GUI 关于页与 CLI `--version` 共用）。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 全部客户端 id。
pub fn client_ids() -> Vec<&'static str> {
    adapters::all().iter().map(|a| a.id()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sandbox(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "mcphub-app-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_catalog(dir: &Path) -> PathBuf {
        let p = dir.join("catalog.yaml");
        fs::write(
            &p,
            r#"
version: 1
entries:
  - name: github
    transport: http
    url: https://api.githubcopilot.com/mcp/
    auth: { type: bearer, token_env: GITHUB_PAT }
  - name: fs
    transport: stdio
    command: npx
    args: ["-y", "srv"]
"#,
        )
        .unwrap();
        p
    }

    fn app_in(dir: &Path) -> App {
        let env = HostEnv {
            home: dir.to_path_buf(),
            appdata: Some(dir.join("AppData/Roaming")),
            project_root: Some(dir.join("proj")),
        };
        App::with_home(dir.join(".mcp-hub"), env, Platform::Windows)
    }

    #[test]
    fn plan_produces_targets_for_all_clients() {
        let dir = sandbox("plan");
        let cat = write_catalog(&dir);
        let app = app_in(&dir);

        let mut req = PlanRequest::new(cat);
        req.clients = Some(vec!["claude-code".into(), "codex".into()]);
        let out = app.plan(&req).unwrap();

        assert_eq!(out.plans.len(), 2);
        assert_eq!(out.entry_count, 2);
        assert!(out.plans.iter().all(|p| p.has_write()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_client_is_skipped_with_reason() {
        let dir = sandbox("skip");
        let cat = write_catalog(&dir);
        let app = app_in(&dir);
        let mut req = PlanRequest::new(cat);
        req.clients = Some(vec!["claude-code".into()]);
        req.scope = Scope::Local;

        let out = app.plan(&req).unwrap();
        assert!(out.plans.is_empty());
        assert_eq!(out.skipped.len(), 1);
        assert!(out.skipped[0].1.contains("本地级"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_scope_filters_entries() {
        let dir = sandbox("scope");
        let p = dir.join("c.yaml");
        fs::write(
            &p,
            r#"
version: 1
entries:
  - { name: a, transport: stdio, command: npx }
  - { name: b, transport: stdio, command: npx, scope: project }
"#,
        )
        .unwrap();
        let app = app_in(&dir);

        let mut user = PlanRequest::new(p.clone());
        user.scope = Scope::User;
        user.clients = Some(vec!["claude-code".into()]);
        assert_eq!(app.plan(&user).unwrap().entry_count, 1);

        // 切到项目级后，未声明 scope 的条目（inherit）也跟随全局变成 project，
        // 因此 a 与 b 都参与；这是「inherit 跟随全局」的直接结果，不是 bug。
        let mut project = PlanRequest::new(p);
        project.scope = Scope::Project;
        project.clients = Some(vec!["claude-code".into()]);
        assert_eq!(app.plan(&project).unwrap().entry_count, 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_then_reapply_is_idempotent() {
        let dir = sandbox("apply");
        let cat = write_catalog(&dir);
        let app = app_in(&dir);
        let mut req = PlanRequest::new(cat);
        req.clients = Some(vec!["claude-code".into()]);

        let out = app.plan(&req).unwrap();
        let receipt = app.apply(&out.plans, false).unwrap();
        assert_eq!(receipt.written(), 1);

        let out2 = app.plan(&req).unwrap();
        assert_eq!(out2.writable(), 0, "第二次应无写入");
        let receipt2 = app.apply(&out2.plans, false).unwrap();
        assert_eq!(receipt2.written(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_client_errors() {
        let dir = sandbox("unknown");
        let cat = write_catalog(&dir);
        let app = app_in(&dir);
        let mut req = PlanRequest::new(cat);
        req.clients = Some(vec!["nope".into()]);
        assert!(matches!(app.plan(&req), Err(AppError::UnknownClient(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dry_run_does_not_write() {
        let dir = sandbox("dry");
        let cat = write_catalog(&dir);
        let app = app_in(&dir);
        let mut req = PlanRequest::new(cat);
        req.clients = Some(vec!["claude-code".into()]);

        let out = app.plan(&req).unwrap();
        let receipt = app.apply(&out.plans, true).unwrap();
        assert_eq!(receipt.written(), 0);
        assert!(!dir.join(".claude.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
