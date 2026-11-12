//! # mcp-app
//!
//! 应用服务层：**宿主无关**的用例编排。
//!
//! GUI（Tauri）与 CLI 都只调用这里，不直接碰 `mcp-io` / `mcp-core`。
//! 因此换前端不需要改动任何业务逻辑（见架构文档 §3.2）。

pub mod catalog_session;
pub mod detect;
pub mod distribution;
pub mod env;
pub mod importing;
pub mod inspect;
mod planning;
pub mod probe;
pub mod settings;

use mcp_core::adapters::{self, Adapter, ClientKind, CustomAdapter};
use mcp_core::interpolate::VarCtx;
use mcp_core::model::{Catalog, McpEntry, Platform, Scope};
use mcp_core::plan::{plan_file_with_options, FilePlan, MergeMode, PlanFileOptions};
use mcp_core::{catalog, resolve};
use mcp_io::{ApplyOptions, ApplyReceipt, BackupManifest, Engine};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use detect::{detect_all, detect_client, DetectState};
pub use env::HostEnv;

// ---------------------------------------------------------------- 适配器注册集合

/// 内置 14 个客户端 + 可选 `<mcp_hub_home>/adapters.yaml` 中的声明式自定义客户端。
#[derive(Default)]
pub struct AdapterSet {
    custom: Vec<CustomAdapter>,
}

impl AdapterSet {
    pub fn by_id<'a>(&'a self, id: &str) -> Option<&'a dyn Adapter> {
        if let Some(builtin) = adapters::by_id(id) {
            return Some(builtin);
        }
        self.custom
            .iter()
            .find(|c| c.id() == id)
            .map(|c| c as &dyn Adapter)
    }

    pub fn all<'a>(&'a self) -> Vec<&'a dyn Adapter> {
        let mut list: Vec<&'a dyn Adapter> = adapters::all().to_vec();
        list.extend(self.custom.iter().map(|c| c as &dyn Adapter));
        list
    }

    pub fn ids(&self) -> Vec<String> {
        self.all().into_iter().map(|a| a.id().to_string()).collect()
    }
}

/// 展开后的物理配置文件路径与容器键路径（含 `#fragment` 动态前缀，如 `projects.<proj>.mcpServers`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetLocation {
    pub path: PathBuf,
    pub root_key: Vec<String>,
}

fn normalize_project_key_for_match(key: &str) -> String {
    let mut s = key.replace('\\', "/");
    while s.len() > 3 && s.ends_with('/') {
        s.pop();
    }
    #[cfg(windows)]
    {
        s = s.to_lowercase();
    }
    s
}

pub(crate) fn compute_root_key(
    adapter: &dyn Adapter,
    scope: Scope,
    target: &env::ExpandedTarget,
) -> Vec<String> {
    let mut root = target.container_prefix.clone();
    // Claude Code 在 `~/.claude.json` 的 `projects` 下可能已按自身分隔符风格保存了项目键；
    // 若磁盘中已存在等价路径键，复用原键名避免产生重复项目节点。
    if root.len() >= 2 && root[0] == "projects" && target.path.is_file() {
        if let Ok(text) = std::fs::read_to_string(&target.path) {
            if let Ok(doc) = mcp_core::document::ConfigDoc::parse(
                adapter.format_for_path(&target.path),
                text.trim_start_matches('\u{feff}'),
            ) {
                let existing_keys = doc.names(&["projects".to_string()]);
                let want = normalize_project_key_for_match(&root[1]);
                if let Some(matched) = existing_keys
                    .into_iter()
                    .find(|k| normalize_project_key_for_match(k) == want)
                {
                    root[1] = matched;
                }
            }
        }
    }
    root.extend(adapter.root_key_for_path(scope, &target.path));
    root
}

// ---------------------------------------------------------------- 作用域能力

/// 检查内置客户端在静态层面是否支持指定作用域。
pub fn scope_block_reason(client_id: &str, scope: Scope) -> Option<String> {
    let adapter = adapters::by_id(client_id)?;
    if !adapter.supports_scope(scope) {
        return Some(format!("{} 不支持{}作用域", adapter.display_name(), scope));
    }
    None
}

/// 结合宿主环境检查任意适配器（含自定义适配器）的作用域可用性。
pub fn scope_block_reason_for(
    adapter: &dyn Adapter,
    scope: Scope,
    env: &HostEnv,
) -> Option<String> {
    if !adapter.supports_scope(scope) {
        return Some(format!("{} 不支持{}作用域", adapter.display_name(), scope));
    }
    if scope == Scope::Local && env.project_root.is_none() {
        return Some("本地级配置需要先选择项目目录".into());
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

    /// 自定义适配器声明文件路径（只读探测时不创建目录）。
    pub fn custom_adapters_path(&self) -> PathBuf {
        self.engine.home().join("adapters.yaml")
    }

    /// 加载内置 + 自定义适配器集合。
    pub fn adapter_set(&self) -> Result<AdapterSet, AppError> {
        let Some(text) = self.engine.read_text(&self.custom_adapters_path())? else {
            return Ok(AdapterSet::default());
        };
        let custom = adapters::parse_custom_adapters(&text)?;
        Ok(AdapterSet { custom })
    }

    pub fn check_scope_block(&self, adapter: &dyn Adapter, scope: Scope) -> Option<String> {
        scope_block_reason_for(adapter, scope, &self.env)
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
        let (cat, issues) = if !req.catalog_path.exists() && req.is_removal_only() {
            (Catalog::default(), Vec::new())
        } else {
            self.load_catalog(&req.catalog_path)?
        };
        self.plan_catalog(req, cat, issues)
    }

    /// 使用已经读取的同一份清单编译，防止预览快照与实际解析读取两次文件。
    pub(crate) fn plan_catalog(
        &self,
        req: &PlanRequest,
        mut cat: Catalog,
        issues: Vec<String>,
    ) -> Result<PlanOutcome, AppError> {
        let adapter_set = self.adapter_set()?;
        let managed_state = self.engine.load_managed_state();
        cat.entries = resolve::entries_for_scope(&cat.entries, req.scope, req.scope);

        let ids: Vec<String> = match &req.clients {
            Some(list) => list.clone(),
            None => {
                if req.selection.is_some() || req.removals.is_some() {
                    let mut set = std::collections::BTreeSet::new();
                    if let Some(selection) = &req.selection {
                        set.extend(selection.keys().cloned());
                    }
                    if let Some(removals) = &req.removals {
                        set.extend(removals.keys().cloned());
                    }
                    set.into_iter().collect()
                } else {
                    adapter_set.ids()
                }
            }
        };

        if let Some(selection) = &req.selection {
            for (id, names) in selection {
                adapter_set
                    .by_id(id)
                    .ok_or_else(|| AppError::UnknownClient(id.clone()))?;
                if !ids.contains(id) {
                    return Err(AppError::InvalidSelection(format!(
                        "客户端 {id} 不在本次目标范围内"
                    )));
                }
                for name in names {
                    if !cat.entries.iter().any(|entry| &entry.name == name) {
                        return Err(AppError::InvalidSelection(format!(
                            "MCP {name} 不存在或不属于当前作用域"
                        )));
                    }
                }
            }
            // 先过滤再解析变量，避免未选中条目的变量告警进入本次计划。
            cat.entries
                .retain(|entry| selection.values().any(|names| names.contains(&entry.name)));
        } else if req.removals.is_some() {
            cat.entries.clear();
        }

        if let Some(removals) = &req.removals {
            for id in removals.keys() {
                adapter_set
                    .by_id(id)
                    .ok_or_else(|| AppError::UnknownClient(id.clone()))?;
                if !ids.contains(id) {
                    return Err(AppError::InvalidSelection(format!(
                        "客户端 {id} 不在本次目标范围内"
                    )));
                }
            }
        }

        let entry_count = cat.entries.len();
        let ctx = self.var_ctx();
        let mut all_missing = Vec::new();
        let mut plans = Vec::new();
        let mut skipped = Vec::new();

        for id in &ids {
            let Some(adapter) = adapter_set.by_id(id) else {
                return Err(AppError::UnknownClient(id.clone()));
            };

            let raw_client_entries: Vec<&McpEntry> = cat
                .entries
                .iter()
                .filter(|entry| match &req.selection {
                    Some(selection) => selection
                        .get(id)
                        .is_some_and(|names| names.contains(&entry.name)),
                    None => req.removals.is_none(),
                })
                .collect();
            let client_removals: &[String] = req
                .removals
                .as_ref()
                .and_then(|r| r.get(id))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if (req.selection.is_some() || req.removals.is_some())
                && raw_client_entries.is_empty()
                && client_removals.is_empty()
            {
                continue;
            }

            if let Some(reason) = self.check_scope_block(adapter, req.scope) {
                skipped.push((id.clone(), reason));
                continue;
            }

            let Some(loc) = self.target_location(adapter, req.scope) else {
                skipped.push((id.clone(), "无法解析配置路径（缺少环境变量）".into()));
                continue;
            };

            let mut client_entries = Vec::with_capacity(raw_client_entries.len());
            for entry in raw_client_entries {
                let (resolved, missing) =
                    resolve::resolve_for_client(entry, adapter, req.scope, self.platform, &ctx);
                all_missing.extend(missing);
                client_entries.push(resolved);
            }

            let existing = self
                .engine
                .read_text(&loc.path)
                .map_err(|e| AppError::Other(e.to_string()))?;

            let managed_entries = mcp_io::managed_container_key(&loc.path, &loc.root_key)
                .ok()
                .and_then(|k| managed_state.containers.get(&k));

            let plan = plan_file_with_options(
                adapter,
                loc.path.display().to_string(),
                req.scope,
                existing.as_deref(),
                &client_entries,
                req.mode,
                req.allow_comment_strip,
                &PlanFileOptions {
                    root_key_override: Some(&loc.root_key),
                    managed_entries,
                    removals: client_removals,
                },
            )?;
            plans.push(plan);
        }

        all_missing.sort_by(|a, b| a.raw.cmp(&b.raw));
        all_missing.dedup_by(|a, b| a.raw == b.raw);

        Ok(PlanOutcome {
            catalog_issues: issues,
            missing_vars: all_missing.into_iter().map(|m| m.raw).collect(),
            plans: planning::coalesce(plans)?,
            skipped,
            entry_count,
        })
    }

    /// 目标路径及动态容器根路径：优先已存在者，否则第一个候选。
    pub fn target_location(&self, adapter: &dyn Adapter, scope: Scope) -> Option<TargetLocation> {
        let templates = adapter.path_candidates(scope, self.platform);
        let candidates = self.env.expand_all_targets(&templates, self.platform);
        let chosen = candidates
            .iter()
            .find(|t| t.path.exists())
            .or_else(|| candidates.first())?;
        let root_key = compute_root_key(adapter, scope, chosen);
        Some(TargetLocation {
            path: chosen.path.clone(),
            root_key,
        })
    }

    /// 目标路径：优先已存在者，否则第一个候选。
    pub fn target_path(&self, adapter: &dyn Adapter, scope: Scope) -> Option<PathBuf> {
        self.target_location(adapter, scope).map(|loc| loc.path)
    }

    /// 执行写入。
    pub fn apply(&self, plans: &[FilePlan], dry_run: bool) -> Result<ApplyReceipt, AppError> {
        Ok(self.engine.apply(plans, &ApplyOptions { dry_run })?)
    }

    pub fn backups(&self) -> Result<Vec<BackupManifest>, AppError> {
        Ok(self.engine.list_backups()?)
    }

    pub fn save_catalog(&self, path: &Path, cat: &Catalog) -> Result<Vec<String>, AppError> {
        self.save_catalog_checked(path, cat, None)
    }

    pub(crate) fn save_catalog_checked(
        &self,
        path: &Path,
        cat: &Catalog,
        expected: Option<Option<&str>>,
    ) -> Result<Vec<String>, AppError> {
        let issues = catalog::validate(cat);
        if issues.iter().any(|issue| issue.is_error()) {
            return Err(AppError::Other("清单校验失败，请检查条目字段".into()));
        }
        let content = if path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("json"))
        {
            serde_json::to_string_pretty(cat)
                .map_err(|_| AppError::Other("清单序列化失败".into()))?
        } else {
            catalog::to_yaml(cat)?
        };
        let receipt = match expected {
            Some(before) => self
                .engine
                .save_catalog_if_unchanged(path, &content, before)?,
            None => self.engine.save_catalog(path, &content)?,
        };
        if let Some(outcome) = receipt.outcomes.iter().find(|o| {
            matches!(
                o.status,
                mcp_io::OutcomeStatus::Failed | mcp_io::OutcomeStatus::RollbackFailed
            )
        }) {
            return Err(AppError::Other(format!(
                "{}；备份 ID：{}",
                outcome.message.as_deref().unwrap_or("清单保存失败"),
                receipt.backup_id.as_deref().unwrap_or("无")
            )));
        }
        let mut warnings: Vec<_> = issues.iter().map(ToString::to_string).collect();
        warnings.extend(
            receipt
                .outcomes
                .iter()
                .filter(|o| o.status == mcp_io::OutcomeStatus::AuditFailed)
                .filter_map(|o| o.message.clone()),
        );
        Ok(warnings)
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
    pub client_details: Vec<(String, String, ClientKind)>,
    pub rows: Vec<MatrixRow>,
    pub issues: Vec<String>,
    /// 零密钥环境变量就绪态：仅记录清单中引用的环境变量在当前宿主环境是否已配置（布尔值，绝不含变量值）。
    pub env_status: BTreeMap<String, bool>,
}

fn collect_vars_in_str(text: &str, out: &mut std::collections::BTreeSet<String>) {
    let mut rest = text;
    while !rest.is_empty() {
        let prefix = ["${ENV:", "${env:", "{env:", "${"]
            .into_iter()
            .find(|p| rest.starts_with(*p));
        if let Some(prefix) = prefix {
            if let Some(end) = rest.find('}') {
                let name = rest[prefix.len()..end].trim();
                if !name.is_empty()
                    && !matches!(name, "HOME" | "HOME_DIR" | "PROJECT_ROOT" | "CWD")
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    out.insert(name.to_string());
                }
                rest = &rest[end + 1..];
                continue;
            }
            break;
        }
        let ch = rest.chars().next().unwrap();
        rest = &rest[ch.len_utf8()..];
    }
}

fn collect_entry_env_vars(entry: &McpEntry) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    if let Some(s) = &entry.command {
        collect_vars_in_str(s, &mut out);
    }
    for arg in &entry.args {
        collect_vars_in_str(arg, &mut out);
    }
    if let Some(s) = &entry.cwd {
        collect_vars_in_str(s, &mut out);
    }
    if let Some(s) = &entry.url {
        collect_vars_in_str(s, &mut out);
    }
    for v in entry.env.values() {
        collect_vars_in_str(v, &mut out);
    }
    for v in entry.headers.values() {
        collect_vars_in_str(v, &mut out);
    }
    if let Some(var) = entry.auth.as_ref().and_then(|a| a.env_name()) {
        out.insert(var.to_string());
    }
    for ov in entry.overrides.values() {
        if let Some(s) = &ov.command {
            collect_vars_in_str(s, &mut out);
        }
        if let Some(args) = &ov.args {
            for arg in args {
                collect_vars_in_str(arg, &mut out);
            }
        }
        if let Some(s) = &ov.cwd {
            collect_vars_in_str(s, &mut out);
        }
        if let Some(s) = &ov.url {
            collect_vars_in_str(s, &mut out);
        }
        if let Some(env_map) = &ov.env {
            for v in env_map.values() {
                collect_vars_in_str(v, &mut out);
            }
        }
        if let Some(hdr_map) = &ov.headers {
            for v in hdr_map.values() {
                collect_vars_in_str(v, &mut out);
            }
        }
        if let Some(var) = ov.auth.as_ref().and_then(|a| a.env_name()) {
            out.insert(var.to_string());
        }
    }
    out
}

impl App {
    /// 生成矩阵视图：每格是「该条目在该客户端」的真实状态。
    ///
    /// 只读：不会写入任何文件。结合零密钥 `ManagedState` 执行 3-Way 对账，
    /// 使清单中已删除的键立刻体现为 `CellState::Diff` 而非误报 `Ok`。
    pub fn matrix(&self, catalog_path: &Path, scope: Scope) -> Result<MatrixView, AppError> {
        let (cat, issues) = self.load_catalog(catalog_path)?;
        let adapter_set = self.adapter_set()?;
        let managed_state = self.engine.load_managed_state();
        let ctx = self.var_ctx();
        let entries = resolve::entries_for_scope(&cat.entries, scope, scope);

        let adapters_list = adapter_set.all();
        let clients: Vec<String> = adapters_list.iter().map(|a| a.id().to_string()).collect();
        let client_details: Vec<(String, String, ClientKind)> = adapters_list
            .iter()
            .map(|a| (a.id().to_string(), a.display_name().to_string(), a.kind()))
            .collect();
        let mut rows = Vec::with_capacity(entries.len());

        // 每个客户端的文档与已托管键骨架只解析一次
        let mut docs: Vec<Option<mcp_core::document::ConfigDoc>> =
            Vec::with_capacity(adapters_list.len());
        let mut roots: Vec<Option<Vec<String>>> = Vec::with_capacity(adapters_list.len());
        let mut managed_maps: Vec<
            Option<BTreeMap<String, mcp_core::document::ManagedKeys>>,
        > = Vec::with_capacity(adapters_list.len());

        for adapter in &adapters_list {
            if self.check_scope_block(*adapter, scope).is_some() {
                docs.push(None);
                roots.push(None);
                managed_maps.push(None);
                continue;
            }
            let Some(loc) = self.target_location(*adapter, scope) else {
                docs.push(None);
                roots.push(None);
                managed_maps.push(None);
                continue;
            };
            let text = self
                .engine
                .read_text(&loc.path)
                .map_err(|e| AppError::Other(e.to_string()))?
                .unwrap_or_default();
            let doc = mcp_core::document::ConfigDoc::parse(
                adapter.format_for_path(&loc.path),
                text.trim_start_matches('\u{feff}'),
            )
            .ok();
            let managed_map = mcp_io::managed_container_key(&loc.path, &loc.root_key)
                .ok()
                .and_then(|k| managed_state.containers.get(&k).cloned());
            docs.push(doc);
            roots.push(Some(loc.root_key));
            managed_maps.push(managed_map);
        }

        for entry in &entries {
            let mut cells = Vec::with_capacity(adapters_list.len());
            for (i, adapter) in adapters_list.iter().enumerate() {
                let id = adapter.id();
                if let Some(reason) = self.check_scope_block(*adapter, scope) {
                    cells.push(MatrixCell {
                        client_id: id.to_string(),
                        state: CellState::Unsupported,
                        detail: Some(reason),
                    });
                    continue;
                }
                let (Some(doc), Some(root)) = (&docs[i], &roots[i]) else {
                    cells.push(MatrixCell {
                        client_id: id.to_string(),
                        state: CellState::Unsupported,
                        detail: Some("无法解析配置路径".into()),
                    });
                    continue;
                };
                let (resolved_entry, _) =
                    resolve::resolve_for_client(entry, *adapter, scope, self.platform, &ctx);
                let encoded = adapter.encode_entry_checked(&resolved_entry)?;
                let state = match doc.get(root, &entry.name) {
                    None => CellState::Absent,
                    Some(existing) => {
                        if !encoded.unresolved_inline_secrets.is_empty() {
                            CellState::Diff
                        } else {
                            let prev_managed = managed_maps[i]
                                .as_ref()
                                .and_then(|m| m.get(&entry.name));
                            let (merged, _) = mcp_core::document::reconcile_entry(
                                Some(&existing),
                                &encoded.value,
                                MergeMode::Merge,
                                prev_managed,
                            );
                            if merged == existing {
                                CellState::Ok
                            } else {
                                CellState::Diff
                            }
                        }
                    }
                };
                cells.push(MatrixCell {
                    client_id: id.to_string(),
                    state,
                    detail: None,
                });
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

        let mut env_status = BTreeMap::new();
        for entry in &cat.entries {
            for var in collect_entry_env_vars(entry) {
                env_status
                    .entry(var.clone())
                    .or_insert_with(|| ctx.lookup_var(&var).is_some());
            }
        }

        Ok(MatrixView {
            platform: self.platform,
            scope,
            clients,
            client_details,
            rows,
            issues,
            env_status,
        })
    }
}

// ---------------------------------------------------------------- 请求/响应

#[derive(Debug, Clone, PartialEq)]
pub struct PlanRequest {
    pub catalog_path: PathBuf,
    /// `None` 表示全部客户端
    pub clients: Option<Vec<String>>,
    /// 精确选择：客户端 id → MCP 名称。`None` 保留 CLI 的全清单行为；
    /// `Some(空映射)` 或客户端对应的空列表均不产生写入。
    pub selection: Option<BTreeMap<String, Vec<String>>>,
    /// 从目标客户端移除的 MCP 条目：客户端 id → MCP 名称。
    pub removals: Option<BTreeMap<String, Vec<String>>>,
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
            selection: None,
            removals: None,
            scope: Scope::User,
            mode: MergeMode::Merge,
            allow_comment_strip: false,
        }
    }

    /// 是否仅为移除操作（不含任何新增/覆盖选择）。此时即便尚未创建 `catalog.yaml` 也允许安全移除已有配置。
    pub fn is_removal_only(&self) -> bool {
        let no_writes = self
            .selection
            .as_ref()
            .is_some_and(|s| s.values().all(Vec::is_empty));
        let has_removals = self
            .removals
            .as_ref()
            .is_some_and(|r| r.values().any(|v| !v.is_empty()));
        no_writes && has_removals
    }
}

#[derive(Debug, Clone, PartialEq)]
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
    #[error("无效选择：{0}")]
    InvalidSelection(String),
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
        let mut req = PlanRequest::new(cat.clone());
        req.clients = Some(vec!["codex".into()]);
        req.scope = Scope::Local;

        let out = app.plan(&req).unwrap();
        assert!(out.plans.is_empty());
        assert_eq!(out.skipped.len(), 1);
        assert!(out.skipped[0].1.contains("本地"));

        // Claude Code 在未选择项目目录时同样跳过本地级；有项目目录时写入 .claude.json#projects.<proj>
        let mut app_no_proj = app_in(&dir);
        app_no_proj.env.project_root = None;
        let mut req_local = PlanRequest::new(cat);
        req_local.clients = Some(vec!["claude-code".into()]);
        req_local.scope = Scope::Local;
        let out_no_proj = app_no_proj.plan(&req_local).unwrap();
        assert!(out_no_proj.plans.is_empty());
        assert!(out_no_proj.skipped[0].1.contains("本地级"));

        let out_with_proj = app.plan(&req_local).unwrap();
        assert_eq!(out_with_proj.plans.len(), 1);
        assert_eq!(out_with_proj.plans[0].root_key[0], "projects");
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
