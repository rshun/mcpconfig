//! IPC 数据传输对象（前端唯一的耦合面）。
//!
//! 刻意与内部类型解耦：前端只依赖这些结构，内部重构不影响 UI。

use mcp_app::detect::{ClientStatus, DetectState};
use mcp_app::inspect::ExistingConfig;
use mcp_app::{CellState, MatrixView};
use mcp_core::plan::{ChangeKind, FilePlan};
use mcp_io::{ApplyReceipt, OutcomeStatus};
use serde::Serialize;

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
}

impl From<MatrixView> for MatrixDto {
    fn from(m: MatrixView) -> Self {
        let clients = m
            .clients
            .iter()
            .map(|id| {
                let a = mcp_core::adapters::by_id(id).unwrap();
                ClientBriefDto {
                    id: a.id().to_string(),
                    name: a.display_name().to_string(),
                    kind: a.kind().to_string(),
                }
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
    pub unchanged_count: usize,
    pub diff: String,
    pub warnings: Vec<String>,
    pub will_write: bool,
}

impl From<FilePlan> for FileDto {
    fn from(p: FilePlan) -> Self {
        let (new_count, update_count) = p.changed_count();
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
    pub entry_count: usize,
    pub issues: Vec<String>,
    pub missing_vars: Vec<String>,
    pub skipped: Vec<SkippedDto>,
    pub files: Vec<FileDto>,
    pub writable: usize,
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
    /// 原样返回清单（前端编辑后回传）
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
