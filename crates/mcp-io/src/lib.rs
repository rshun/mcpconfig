//! # mcp-io
//!
//! 写入引擎：**备份 → 原子写 → 写后校验 → 失败回滚 → 审计**。
//!
//! 这是唯一会改动用户文件的地方，因此每条路径都必须可回退。

pub mod time;

use mcp_core::adapters::Format;
use mcp_core::document::ConfigDoc;
use mcp_core::plan::FilePlan;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use time::UtcTime;

// ---------------------------------------------------------------- 错误

#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error("IO 失败：{0}")]
    Io(String),
    #[error("写入后校验失败，已回滚：{0}")]
    VerifyFailed(String),
    #[error("未找到备份：{0}")]
    BackupNotFound(String),
    #[error("备份已损坏或不完整：{0}")]
    BackupCorrupt(String),
}

pub type Result<T> = std::result::Result<T, IoError>;

fn io_err(e: impl std::fmt::Display) -> IoError {
    IoError::Io(e.to_string())
}

// ---------------------------------------------------------------- 选项与回执

#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    /// 只计算不落盘。
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeStatus {
    /// 已写入
    Written,
    /// 无需写入（内容一致）
    Unchanged,
    /// 被阻止（作用域不支持 / 解析失败 / 含注释等）
    Blocked,
    /// 写入失败并已回滚
    Failed,
    /// 演练模式
    DryRun,
}

impl OutcomeStatus {
    pub fn label(self) -> &'static str {
        match self {
            OutcomeStatus::Written => "已写入",
            OutcomeStatus::Unchanged => "无变化",
            OutcomeStatus::Blocked => "已跳过",
            OutcomeStatus::Failed => "失败",
            OutcomeStatus::DryRun => "演练",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileOutcome {
    pub client_id: String,
    pub path: String,
    pub status: OutcomeStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ApplyReceipt {
    /// 本次事务的备份 id（有实际写入时才存在）
    pub backup_id: Option<String>,
    pub outcomes: Vec<FileOutcome>,
    pub audit_path: Option<PathBuf>,
}

impl ApplyReceipt {
    pub fn written(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.status == OutcomeStatus::Written)
            .count()
    }
    pub fn failed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.status == OutcomeStatus::Failed)
            .count()
    }
    pub fn blocked(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.status == OutcomeStatus::Blocked)
            .count()
    }
    pub fn all_ok(&self) -> bool {
        self.failed() == 0
    }
}

// ---------------------------------------------------------------- 备份清单

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupFileEntry {
    pub client_id: String,
    pub scope: String,
    /// 原文件绝对路径
    pub path: String,
    /// 备份目录内的文件名
    pub stored: String,
    /// 写入前是否存在（false 表示本次是新建，回滚时应删除）
    pub existed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub id: String,
    pub created_at: String,
    pub files: Vec<BackupFileEntry>,
}

impl BackupManifest {
    pub fn total_bytes(&self, dir: &Path) -> u64 {
        self.files
            .iter()
            .filter_map(|f| fs::metadata(dir.join(&f.stored)).ok())
            .map(|m| m.len())
            .sum()
    }
}

// ---------------------------------------------------------------- 引擎

pub struct Engine {
    home: PathBuf,
}

impl Engine {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    /// 默认位置：`~/.mcp-hub`。
    pub fn with_default_home() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        Self::new(PathBuf::from(home).join(".mcp-hub"))
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.home.join("backups")
    }

    pub fn audit_path(&self) -> PathBuf {
        self.home.join("audit.log")
    }

    /// 读取文件；不存在返回 `None`（不算错误）。
    pub fn read_text(&self, path: &Path) -> Result<Option<String>> {
        match fs::read_to_string(path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(format!("读取 {} 失败：{e}", path.display()))),
        }
    }

    /// 执行计划。按文件独立事务：单个失败不影响其他文件。
    pub fn apply(&self, plans: &[FilePlan], opts: &ApplyOptions) -> Result<ApplyReceipt> {
        let mut receipt = ApplyReceipt::default();

        // 分类
        let mut writable: Vec<&FilePlan> = Vec::new();
        for p in plans {
            if let Some(reason) = &p.blocked {
                receipt.outcomes.push(FileOutcome {
                    client_id: p.client_id.clone(),
                    path: p.path.clone(),
                    status: OutcomeStatus::Blocked,
                    message: Some(reason.clone()),
                });
            } else if !p.has_write() {
                receipt.outcomes.push(FileOutcome {
                    client_id: p.client_id.clone(),
                    path: p.path.clone(),
                    status: OutcomeStatus::Unchanged,
                    message: None,
                });
            } else {
                writable.push(p);
            }
        }

        if writable.is_empty() {
            return Ok(receipt);
        }

        if opts.dry_run {
            for p in writable {
                receipt.outcomes.push(FileOutcome {
                    client_id: p.client_id.clone(),
                    path: p.path.clone(),
                    status: OutcomeStatus::DryRun,
                    message: None,
                });
            }
            return Ok(receipt);
        }

        // 1) 先为所有待写文件建备份（写入前完成，失败可整体感知）
        let now = UtcTime::now();
        let backup_id = now.stamp();
        let backup_dir = self.backups_dir().join(&backup_id);
        fs::create_dir_all(&backup_dir).map_err(io_err)?;
        restrict_permissions(&backup_dir, true)?;

        let mut manifest = BackupManifest {
            id: backup_id.clone(),
            created_at: now.rfc3339(),
            files: Vec::new(),
        };

        for (idx, p) in writable.iter().enumerate() {
            let existed = Path::new(&p.path).exists();
            let stored = format!(
                "{:02}-{}{}",
                idx,
                sanitize(&p.client_id),
                extension_of(&p.path)
            );
            if existed {
                let src = Path::new(&p.path);
                let dst = backup_dir.join(&stored);
                fs::copy(src, &dst)
                    .map_err(|e| io_err(format!("备份 {} 失败：{e}", src.display())))?;
                restrict_permissions(&dst, false)?;
            }
            manifest.files.push(BackupFileEntry {
                client_id: p.client_id.clone(),
                scope: p.scope.as_str().to_string(),
                path: p.path.clone(),
                stored,
                existed,
            });
        }

        let manifest_path = backup_dir.join("manifest.json");
        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| io_err(format!("备份清单序列化失败：{e}")))?;
        fs::write(&manifest_path, manifest_json).map_err(io_err)?;
        receipt.backup_id = Some(backup_id.clone());

        // 2) 逐个写入 + 校验，失败立即回滚该文件
        for (idx, p) in writable.iter().enumerate() {
            let entry = &manifest.files[idx];
            match self.write_and_verify(p) {
                Ok(()) => receipt.outcomes.push(FileOutcome {
                    client_id: p.client_id.clone(),
                    path: p.path.clone(),
                    status: OutcomeStatus::Written,
                    message: None,
                }),
                Err(e) => {
                    // 回滚该文件
                    let path = Path::new(&p.path);
                    if entry.existed {
                        let src = backup_dir.join(&entry.stored);
                        let _ = fs::copy(&src, path);
                    } else {
                        let _ = fs::remove_file(path);
                    }
                    receipt.outcomes.push(FileOutcome {
                        client_id: p.client_id.clone(),
                        path: p.path.clone(),
                        status: OutcomeStatus::Failed,
                        message: Some(format!("{e}")),
                    });
                }
            }
        }

        // 3) 审计
        if let Err(e) = self.append_audit(&manifest, &receipt) {
            // 审计失败不应让用户丢失已成功的写入，只在回执里提示
            receipt.outcomes.push(FileOutcome {
                client_id: "-".into(),
                path: self.audit_path().display().to_string(),
                status: OutcomeStatus::Failed,
                message: Some(format!("审计日志写入失败：{e}")),
            });
        } else {
            receipt.audit_path = Some(self.audit_path());
        }

        Ok(receipt)
    }

    fn write_and_verify(&self, plan: &FilePlan) -> Result<()> {
        let path = Path::new(&plan.path);
        atomic_write(path, &plan.new_content)?;

        // 写后校验：重新解析
        let written = fs::read_to_string(path).map_err(io_err)?;
        match ConfigDoc::parse(plan.format, &written) {
            Ok(_) => Ok(()),
            Err(e) => Err(IoError::VerifyFailed(format!("{}：{e}", plan.path))),
        }
    }

    fn append_audit(&self, manifest: &BackupManifest, receipt: &ApplyReceipt) -> Result<()> {
        if let Some(parent) = self.audit_path().parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.audit_path())
            .map_err(io_err)?;
        restrict_permissions(&self.audit_path(), false)?;

        let ts = UtcTime::now().rfc3339();
        for (idx, o) in receipt.outcomes.iter().enumerate() {
            let stored = manifest.files.get(idx).map(|f| f.stored.clone());
            let line = format!(
                "{ts}\t{}\t{}\t{}\t{}\t{}\n",
                status_tag(o.status),
                o.client_id,
                o.path,
                stored.unwrap_or_else(|| "-".into()),
                o.message.clone().unwrap_or_default().replace('\n', " ")
            );
            f.write_all(line.as_bytes()).map_err(io_err)?;
        }
        f.flush().map_err(io_err)?;
        Ok(())
    }

    /// 列出全部备份（按 id 倒序）。
    pub fn list_backups(&self) -> Result<Vec<BackupManifest>> {
        let dir = self.backups_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir).map_err(io_err)? {
            let entry = entry.map_err(io_err)?;
            let manifest = entry.path().join("manifest.json");
            if !manifest.exists() {
                continue;
            }
            let text = fs::read_to_string(&manifest).map_err(io_err)?;
            match serde_json::from_str::<BackupManifest>(&text) {
                Ok(m) => out.push(m),
                Err(_) => continue, // 损坏的备份目录跳过
            }
        }
        out.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(out)
    }

    /// 回滚到指定备份。返回恢复的文件数。
    pub fn restore(&self, backup_id: &str) -> Result<usize> {
        let dir = self.backups_dir().join(backup_id);
        let manifest_path = dir.join("manifest.json");
        if !manifest_path.exists() {
            return Err(IoError::BackupNotFound(backup_id.to_string()));
        }
        let text = fs::read_to_string(&manifest_path).map_err(io_err)?;
        let manifest: BackupManifest = serde_json::from_str(&text)
            .map_err(|e| IoError::BackupCorrupt(format!("{backup_id}：{e}")))?;

        let mut restored = 0usize;
        for f in &manifest.files {
            let target = Path::new(&f.path);
            if f.existed {
                let src = dir.join(&f.stored);
                if !src.exists() {
                    return Err(IoError::BackupCorrupt(format!(
                        "{} 缺失备份文件 {}",
                        backup_id, f.stored
                    )));
                }
                let content = fs::read_to_string(&src).map_err(io_err)?;
                atomic_write(target, &content)?;
                restored += 1;
            } else if target.exists() {
                fs::remove_file(target).map_err(io_err)?;
                restored += 1;
            }
        }

        // 回滚本身也进审计
        let _ = self.append_raw_audit(&format!(
            "{}\trollback\t-\t{}\t{}\trestored={restored}\n",
            UtcTime::now().rfc3339(),
            backup_id,
            dir.display()
        ));
        Ok(restored)
    }

    fn append_raw_audit(&self, line: &str) -> Result<()> {
        if let Some(parent) = self.audit_path().parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.audit_path())
            .map_err(io_err)?;
        f.write_all(line.as_bytes()).map_err(io_err)?;
        f.flush().map_err(io_err)?;
        Ok(())
    }
}

// ---------------------------------------------------------------- 工具

fn status_tag(s: OutcomeStatus) -> &'static str {
    match s {
        OutcomeStatus::Written => "write",
        OutcomeStatus::Unchanged => "skip",
        OutcomeStatus::Blocked => "block",
        OutcomeStatus::Failed => "fail",
        OutcomeStatus::DryRun => "dry",
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn extension_of(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".toml") {
        ".toml"
    } else if lower.ends_with(".jsonc") {
        ".jsonc"
    } else if lower.ends_with(".json") {
        ".json"
    } else {
        ".txt"
    }
}

/// 原子写：临时文件 → fsync → rename。
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err)?;
    }

    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "config".into());
    let tmp = path.with_file_name(format!(".{file_name}.tmp-{}-{n}", std::process::id()));

    {
        let mut f = fs::File::create(&tmp).map_err(io_err)?;
        f.write_all(content.as_bytes()).map_err(io_err)?;
        f.sync_all().map_err(io_err)?;
    }

    // 保持原文件权限（若已存在）
    if let Ok(meta) = fs::metadata(path) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }

    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(io_err(format!("原子写入 {} 失败：{e}", path.display())))
        }
    }
}

/// 收紧权限：目录 0700、文件 0600（仅 Unix 生效）。
fn restrict_permissions(path: &Path, is_dir: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if is_dir { 0o700 } else { 0o600 };
        let perms = fs::Permissions::from_mode(mode);
        fs::set_permissions(path, perms).map_err(io_err)?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, is_dir);
    }
    Ok(())
}

/// 解析格式的便捷函数（供上层判断）。
pub fn format_of(path: &str) -> Format {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".toml") {
        Format::Toml
    } else if lower.ends_with(".jsonc") {
        Format::Jsonc
    } else {
        Format::Json
    }
}
