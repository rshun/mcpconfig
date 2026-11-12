//! IPC 数据传输对象（前端唯一的耦合面）。
//!
//! 刻意与内部类型解耦：前端只依赖这些结构，内部重构不影响 UI。

use mcp_app::detect::{ClientStatus, DetectState};
use mcp_app::inspect::ExistingConfig;
use mcp_app::probe::ProbeReport;
use mcp_app::{CellState, MatrixView, PlanOutcome};
use mcp_core::plan::{ChangeKind, FilePlan};
use mcp_io::{ApplyReceipt, OutcomeStatus};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingEntryDto {
    pub name: String,
    pub config: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingConfigDto {
    pub client_id: String,
    pub client_name: String,
    pub scope: String,
    pub path: String,
    pub format: String,
    pub existed: bool,
    pub entries: Vec<ExistingEntryDto>,
}

impl From<ExistingConfig> for ExistingConfigDto {
    fn from(config: ExistingConfig) -> Self {
        Self {
            client_id: config.client_id,
            client_name: config.client_name,
            scope: config.scope.as_str().to_string(),
            path: config.path,
            format: config.format.to_string(),
            existed: config.existed,
            entries: config
                .entries
                .into_iter()
                .map(|entry| ExistingEntryDto {
                    name: entry.name,
                    config: entry.config,
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDto {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub format: String,
    pub state: String,
    pub state_label: String,
    pub binary: Option<String>,
    pub path: Option<String>,
    pub configured: usize,
    pub scopes: Vec<String>,
    pub scope_block: Option<String>,
}

impl From<ClientStatus> for ClientDto {
    fn from(s: ClientStatus) -> Self {
        Self {
            id: s.id,
            name: s.name,
            kind: s.kind.to_string(),
            format: s.format.to_string(),
            state: state_key(s.state).to_string(),
            state_label: s.state.label().to_string(),
            binary: s.binary,
            path: s.chosen_path.map(|p| p.display().to_string()),
            configured: s.configured_entries.len(),
            scopes: s.scopes.iter().map(|x| x.as_str().to_string()).collect(),
            scope_block: s.scope_block,
        }
    }
}

fn state_key(s: DetectState) -> &'static str {
    match s {
        DetectState::Ready => "ready",
        DetectState::InstalledNoConfig => "installed",
        DetectState::ConfigOnly => "configOnly",
        DetectState::NotDetected => "missing",
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellDto {
    pub client_id: String,
    pub state: String,
    pub detail: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowDto {
    pub name: String,
    pub transport: String,
    pub scope: String,
    pub cells: Vec<CellDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientBriefDto {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixDto {
    pub platform: String,
    pub scope: String,
    pub clients: Vec<ClientBriefDto>,
    pub rows: Vec<RowDto>,
    pub issues: Vec<String>,
    pub env_status: BTreeMap<String, bool>,
}

impl From<MatrixView> for MatrixDto {
    fn from(m: MatrixView) -> Self {
        let clients = m
            .client_details
            .into_iter()
            .map(|(id, name, kind)| ClientBriefDto {
                id,
                name,
                kind: kind.to_string(),
            })
            .collect();

        let rows = m
            .rows
            .into_iter()
            .map(|r| RowDto {
                name: r.name,
                transport: r.transport,
                scope: r.scope.as_str().to_string(),
                cells: r
                    .cells
                    .into_iter()
                    .map(|c| CellDto {
                        client_id: c.client_id,
                        state: cell_key(c.state).to_string(),
                        detail: c.detail,
                    })
                    .collect(),
            })
            .collect();

        Self {
            platform: m.platform.as_str().to_string(),
            scope: m.scope.as_str().to_string(),
            clients,
            rows,
            issues: m.issues,
            env_status: m.env_status,
        }
    }
}

fn cell_key(s: CellState) -> &'static str {
    match s {
        CellState::Ok => "ok",
        CellState::Absent => "absent",
        CellState::Diff => "diff",
        CellState::Unsupported => "unsupported",
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDto {
    pub client_id: String,
    pub client_name: String,
    pub path: String,
    pub format: String,
    pub scope: String,
    pub existed: bool,
    pub blocked: Option<String>,
    pub new_count: usize,
    pub update_count: usize,
    pub removed_count: usize,
    pub unchanged_count: usize,
    pub diff: String,
    pub warnings: Vec<String>,
    pub will_write: bool,
}

impl From<FilePlan> for FileDto {
    fn from(p: FilePlan) -> Self {
        let (new_count, update_count) = p.changed_count();
        let removed_count = p.removed_count();
        let diff = p.diff_text();
        let will_write = p.has_write();
        let unchanged_count = p
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Unchanged)
            .count();
        Self {
            client_id: p.client_id,
            client_name: p.client_name,
            path: p.path,
            format: p.format.to_string(),
            scope: p.scope.as_str().to_string(),
            existed: p.existed,
            blocked: p.blocked,
            new_count,
            update_count,
            removed_count,
            unchanged_count,
            diff,
            warnings: p.warnings,
            will_write,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDto {
    pub preview_id: String,
    pub entry_count: usize,
    pub issues: Vec<String>,
    pub missing_vars: Vec<String>,
    pub skipped: Vec<SkippedDto>,
    pub files: Vec<FileDto>,
    pub writable: usize,
}

impl From<(String, PlanOutcome)> for PlanDto {
    fn from((preview_id, out): (String, PlanOutcome)) -> Self {
        let writable = out.writable();
        Self {
            preview_id,
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
        }
    }
}

#[cfg(test)]
mod distribution_tests {
    use super::*;
    #[test]
    fn preview_dto_contains_an_id_and_redacted_diff_but_no_raw_plan() {
        let mut entry = mcp_core::McpEntry::new_stdio(
            "alpha",
            "python",
            vec![
                "--api-key".into(),
                "your_argument_secret_here".into(),
                "--port=8080".into(),
            ],
        );
        entry
            .env
            .insert("API_KEY".into(), "your_new_secret_here".into());
        let plan = mcp_core::plan::plan_file(mcp_core::adapters::by_id("claude-code").unwrap(), "example.json", mcp_core::Scope::User,
            Some(r#"{"unrelated":"your_other_secret_here","mcpServers":{"alpha":{"command":"python","env":{"API_KEY":"your_old_secret_here"}}}}"#),
            &[entry], mcp_core::plan::MergeMode::Merge, false).unwrap();
        let dto = PlanDto::from((
            "preview-example".into(),
            PlanOutcome {
                plans: vec![plan],
                catalog_issues: vec![],
                missing_vars: vec![],
                skipped: vec![],
                entry_count: 1,
            },
        ));
        let value = serde_json::to_value(dto).unwrap();
        assert_eq!(value["previewId"], "preview-example");
        let serialized = value.to_string();
        for secret in [
            "your_argument_secret_here",
            "your_new_secret_here",
            "your_other_secret_here",
            "your_old_secret_here",
        ] {
            assert!(!serialized.contains(secret));
        }
        assert!(value["files"][0].get("oldContent").is_none());
        assert!(value["files"][0].get("newContent").is_none());
        assert!(serialized.contains("--port=8080"));
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedDto {
    pub client_id: String,
    pub reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeDto {
    pub client_id: String,
    pub path: String,
    pub status: String,
    pub label: String,
    pub message: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptDto {
    pub written: usize,
    pub blocked: usize,
    pub failed: usize,
    pub backup_id: Option<String>,
    pub outcomes: Vec<OutcomeDto>,
}

impl From<ApplyReceipt> for ReceiptDto {
    fn from(r: ApplyReceipt) -> Self {
        Self {
            written: r.written(),
            blocked: r.blocked(),
            failed: r.failed(),
            backup_id: r.backup_id,
            outcomes: r
                .outcomes
                .into_iter()
                .map(|o| OutcomeDto {
                    client_id: o.client_id,
                    path: o.path,
                    status: outcome_key(o.status).to_string(),
                    label: o.status.label().to_string(),
                    message: o.message,
                })
                .collect(),
        }
    }
}

fn outcome_key(s: OutcomeStatus) -> &'static str {
    match s {
        OutcomeStatus::Written => "written",
        OutcomeStatus::Unchanged => "unchanged",
        OutcomeStatus::Blocked => "blocked",
        OutcomeStatus::Failed => "failed",
        OutcomeStatus::RollbackFailed => "rollbackFailed",
        OutcomeStatus::AuditFailed => "auditFailed",
        OutcomeStatus::DryRun => "dryRun",
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupDto {
    pub id: String,
    pub created_at: String,
    pub files: usize,
    pub paths: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDto {
    pub exists: bool,
    /// 会话乐观并发修订号（保存时用于 CAS 校验与隐藏凭据回填）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    /// 已脱敏的清单视图（敏感值替换为 `[已隐藏]`，后端保存时自动回填未修改项）
    pub catalog: serde_json::Value,
    pub issues: Vec<String>,
    pub entry_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfoDto {
    pub version: String,
    pub platform: String,
    pub home: String,
    pub project_root: String,
    pub data_dir: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeDto {
    pub ok: bool,
    pub kind: String,
    pub summary: String,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub tools: Vec<String>,
    pub missing_vars: Vec<String>,
    pub duration_ms: u64,
}

impl From<ProbeReport> for ProbeDto {
    fn from(r: ProbeReport) -> Self {
        Self {
            ok: r.ok,
            kind: r.kind,
            summary: r.summary,
            server_name: r.server_name,
            server_version: r.server_version,
            tools: r.tools,
            missing_vars: r.missing_vars,
            duration_ms: r.duration_ms,
        }
    }
}
