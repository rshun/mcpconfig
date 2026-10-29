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
    #[error("目标文件已变化，请重新预览：{0}")]
    Conflict(String),
    #[error("IO 失败：{0}")]
    Io(String),
    #[error("写入后校验失败：{0}")]
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
    /// 写入失败，原状态已经恢复
    Failed,
    /// 写入失败且回滚失败，必须人工检查备份
    RollbackFailed,
    /// 文件已经写入，但审计日志失败
    AuditFailed,
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
            OutcomeStatus::RollbackFailed => "回滚失败",
            OutcomeStatus::AuditFailed => "审计失败",
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
            .filter(|o| {
                matches!(
                    o.status,
                    OutcomeStatus::Failed
                        | OutcomeStatus::RollbackFailed
                        | OutcomeStatus::AuditFailed
                )
            })
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
    backup_root: Option<PathBuf>,
}

impl Engine {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            backup_root: None,
        }
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
        self.backup_root
            .clone()
            .unwrap_or_else(|| self.home.join("backups"))
    }

    pub fn with_backups_dir(mut self, path: PathBuf) -> Self {
        self.backup_root = Some(path);
        self
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

        // 宿主必须先合并共享文件；拒绝绕过应用层传入的重复写入。
        let mut paths = std::collections::BTreeSet::new();
        for plan in plans.iter().filter(|p| p.has_write()) {
            if !paths.insert(file_key(Path::new(&plan.path))?) {
                return Err(io_err("同一文件包含多个写入计划，请先合并后再应用"));
            }
        }

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

        // 1) 所有备份成功落盘后才开始写入。
        let sources: Vec<_> = writable
            .iter()
            .map(|p| (p.client_id.as_str(), p.scope.as_str(), p.path.as_str()))
            .collect();
        let (manifest, backup_dir) = self.backup_files(&sources)?;
        receipt.backup_id = Some(manifest.id.clone());

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
                Err(IoError::Conflict(message)) => receipt.outcomes.push(FileOutcome {
                    client_id: p.client_id.clone(),
                    path: p.path.clone(),
                    status: OutcomeStatus::Blocked,
                    message: Some(format!("目标文件已变化，未写入，请重新预览：{message}")),
                }),
                Err(e) => {
                    receipt.outcomes.push(self.failure_outcome(
                        &p.client_id,
                        &p.path,
                        entry,
                        &backup_dir,
                        e,
                    ));
                }
            }
        }

        // 3) 审计
        if let Err(e) = self.append_audit(&manifest, &receipt) {
            // 审计失败不应让用户丢失已成功的写入，只在回执里提示
            receipt.outcomes.push(FileOutcome {
                client_id: "-".into(),
                path: self.audit_path().display().to_string(),
                status: OutcomeStatus::AuditFailed,
                message: Some(format!("文件写入结果未回滚；审计日志写入失败：{e}")),
            });
        } else {
            receipt.audit_path = Some(self.audit_path());
        }

        Ok(receipt)
    }

    fn write_and_verify(&self, plan: &FilePlan) -> Result<()> {
        let path = Path::new(&plan.path);
        let current = self.read_text(path)?;
        if current.as_deref()
            != if plan.existed {
                Some(plan.old_content.as_str())
            } else {
                None
            }
        {
            return Err(IoError::Conflict(plan.path.clone()));
        }
        atomic_write(path, &plan.new_content)?;

        // 写后校验：重新解析
        let written = fs::read_to_string(path).map_err(io_err)?;
        match ConfigDoc::parse(plan.format, &written) {
            Ok(_) => Ok(()),
            Err(_) => Err(IoError::VerifyFailed(plan.path.clone())),
        }
    }

    fn backup_files(&self, sources: &[(&str, &str, &str)]) -> Result<(BackupManifest, PathBuf)> {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(self.backups_dir()).map_err(io_err)?;
        restrict_permissions(&self.backups_dir(), true)?;
        let (id, dir, created_at) = loop {
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(io_err)?;
            let now = UtcTime::from_unix(time.as_secs());
            let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
            let id = format!(
                "{}-{:09}-{}-{serial:016}",
                now.stamp(),
                time.subsec_nanos(),
                std::process::id()
            );
            let dir = self.backups_dir().join(&id);
            match fs::create_dir(&dir) {
                Ok(()) => break (id, dir, now.rfc3339()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io_err(e)),
            }
        };
        restrict_permissions(&dir, true)?;
        let mut manifest = BackupManifest {
            id,
            created_at,
            files: Vec::new(),
        };
        for (idx, &(client_id, scope, path)) in sources.iter().enumerate() {
            let source = Path::new(path);
            let existed = source.try_exists().map_err(io_err)?;
            let stored = format!("{idx:02}-{}{}", sanitize(client_id), extension_of(path));
            if existed {
                let destination = dir.join(&stored);
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination)
                    .map_err(io_err)?;
                restrict_permissions(&destination, false)?;
                let mut input = fs::File::open(source).map_err(io_err)?;
                std::io::copy(&mut input, &mut output).map_err(io_err)?;
                output.sync_all().map_err(io_err)?;
            }
            manifest.files.push(BackupFileEntry {
                client_id: client_id.into(),
                scope: scope.into(),
                path: path.into(),
                stored,
                existed,
            });
        }
        let manifest_path = dir.join("manifest.json");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&manifest_path)
            .map_err(io_err)?;
        restrict_permissions(&manifest_path, false)?;
        file.write_all(
            serde_json::to_string_pretty(&manifest)
                .map_err(io_err)?
                .as_bytes(),
        )
        .map_err(io_err)?;
        file.sync_all().map_err(io_err)?;
        Ok((manifest, dir))
    }

    fn failure_outcome(
        &self,
        client_id: &str,
        path: &str,
        entry: &BackupFileEntry,
        dir: &Path,
        error: IoError,
    ) -> FileOutcome {
        let rollback = (|| -> Result<()> {
            let target = Path::new(path);
            if entry.existed {
                let content = fs::read_to_string(dir.join(&entry.stored)).map_err(io_err)?;
                atomic_write(target, &content)?;
                if fs::read_to_string(target).map_err(io_err)? != content {
                    return Err(io_err("回滚后的文件内容与备份不一致"));
                }
            } else {
                match fs::remove_file(target) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io_err(e)),
                }
            }
            Ok(())
        })();
        let (status, message) = match rollback {
            Ok(()) => (OutcomeStatus::Failed, format!("{error}；已恢复写入前状态")),
            Err(rollback) => (
                OutcomeStatus::RollbackFailed,
                format!(
                    "{error}；自动回滚失败：{rollback}。请保留并检查备份 {}",
                    dir.display()
                ),
            ),
        };
        FileOutcome {
            client_id: client_id.into(),
            path: path.into(),
            status,
            message: Some(message),
        }
    }

    /// 清单与 Agent 配置遵循同一备份、原子写入、验证和回滚规则。
    pub fn save_catalog(&self, path: &Path, content: &str) -> Result<ApplyReceipt> {
        self.save_checked(path, content, "catalog", None, |text| {
            mcp_core::catalog::load(text).is_ok()
        })
    }

    /// 导入预览绑定目标原文；检查放在备份和写入的同一保存流程内。
    pub fn save_catalog_if_unchanged(
        &self,
        path: &Path,
        content: &str,
        expected: Option<&str>,
    ) -> Result<ApplyReceipt> {
        self.save_checked(path, content, "catalog", Some(expected), |text| {
            mcp_core::catalog::load(text).is_ok()
        })
    }

    pub fn save_settings(&self, path: &Path, content: &str) -> Result<ApplyReceipt> {
        self.save_checked(path, content, "settings", None, |text| {
            serde_json::from_str::<serde_json::Value>(text).is_ok_and(|value| value.is_object())
        })
    }

    fn save_checked(
        &self,
        path: &Path,
        content: &str,
        kind: &str,
        expected: Option<Option<&str>>,
        valid: fn(&str) -> bool,
    ) -> Result<ApplyReceipt> {
        if !valid(content) {
            return Err(io_err("内容校验失败，未写入"));
        }
        let path = std::path::absolute(path).map_err(io_err)?;
        let path_text = path.display().to_string();
        let before = self.read_text(&path)?;
        if expected.is_some_and(|expected| before.as_deref() != expected) {
            return Err(IoError::Conflict("配置库已变化，请重新预览导入".into()));
        }
        if before.as_deref() == Some(content) {
            return Ok(ApplyReceipt::default());
        }
        let (manifest, dir) = self.backup_files(&[(kind, "user", &path_text)])?;
        // 备份期间有其他程序修改清单时中止，不能覆盖新内容。
        if self.read_text(&path)? != before {
            return Err(IoError::Conflict(path_text));
        }
        let write = (|| -> Result<()> {
            atomic_write(&path, content)?;
            let actual = self
                .read_text(&path)?
                .ok_or_else(|| io_err("保存后文件丢失"))?;
            if actual != content || !valid(&actual) {
                return Err(IoError::VerifyFailed("保存内容".into()));
            }
            Ok(())
        })();
        let outcome = match write {
            Ok(()) => FileOutcome {
                client_id: kind.into(),
                path: path_text.clone(),
                status: OutcomeStatus::Written,
                message: None,
            },
            Err(error) => self.failure_outcome(kind, &path_text, &manifest.files[0], &dir, error),
        };
        let mut receipt = ApplyReceipt {
            backup_id: Some(manifest.id.clone()),
            outcomes: vec![outcome],
            ..Default::default()
        };
        if let Err(error) = self.append_audit(&manifest, &receipt) {
            receipt.outcomes.push(FileOutcome {
                client_id: "-".into(),
                path: self.audit_path().display().to_string(),
                status: OutcomeStatus::AuditFailed,
                message: Some(format!("保存结果未回滚；审计写入失败：{error}")),
            });
        }
        Ok(receipt)
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
        for o in &receipt.outcomes {
            let stored = manifest
                .files
                .iter()
                .find(|f| f.path == o.path && f.client_id == o.client_id)
                .map(|f| f.stored.clone());
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

/// 现有文件按真实路径分组；尚未创建的文件解析最近存在的父目录。
pub fn file_key(path: &Path) -> Result<String> {
    let mut parent = std::path::absolute(path).map_err(io_err)?;
    let mut suffix = Vec::new();
    let mut canonical = loop {
        match fs::canonicalize(&parent) {
            Ok(path) => break path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let name = parent
                    .file_name()
                    .ok_or_else(|| io_err("无法解析目标路径"))?
                    .to_os_string();
                suffix.push(name);
                if !parent.pop() {
                    return Err(io_err("无法解析目标父目录"));
                }
            }
            Err(e) => return Err(io_err(e)),
        }
    };
    for part in suffix.into_iter().rev() {
        canonical.push(part);
    }
    let key = canonical.to_string_lossy().to_string();
    #[cfg(windows)]
    let key = key.to_lowercase();
    Ok(key)
}

fn status_tag(s: OutcomeStatus) -> &'static str {
    match s {
        OutcomeStatus::Written => "write",
        OutcomeStatus::Unchanged => "skip",
        OutcomeStatus::Blocked => "block",
        OutcomeStatus::Failed => "fail",
        OutcomeStatus::RollbackFailed => "rollback-failed",
        OutcomeStatus::AuditFailed => "audit-failed",
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
    // 保留用户配置文件的符号链接，替换其实际指向的文件。
    let actual = if fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        fs::canonicalize(path).map_err(io_err)?
    } else {
        path.to_path_buf()
    };
    let path = actual.as_path();
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
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(io_err)?;
        restrict_permissions(&tmp, false)?;
        f.write_all(content.as_bytes()).map_err(io_err)?;
        f.sync_all().map_err(io_err)?;
    }

    // 保持原文件权限（若已存在）
    if let Ok(meta) = fs::metadata(path) {
        fs::set_permissions(&tmp, meta.permissions()).map_err(io_err)?;
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

#[cfg(test)]
mod safety_tests {
    use super::*;

    #[test]
    fn rollback_failure_is_reported_and_backup_is_preserved() {
        let root = std::env::temp_dir().join(format!(
            "mcphub-rollback-failure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = Engine::new(root.join("data"));
        let target = root.join("target.json");
        // 模拟写入失败后目标被目录占用，使新建文件的回滚也失败。
        fs::create_dir_all(&target).unwrap();
        let dir = root.join("backup");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("preserve.txt"), "backup must remain").unwrap();
        let entry = BackupFileEntry {
            client_id: "test".into(),
            scope: "user".into(),
            path: target.display().to_string(),
            stored: "preserve.txt".into(),
            existed: false,
        };
        let outcome = engine.failure_outcome(
            "test",
            &entry.path,
            &entry,
            &dir,
            IoError::VerifyFailed("测试校验".into()),
        );
        assert_eq!(outcome.status, OutcomeStatus::RollbackFailed);
        assert!(outcome.message.as_deref().unwrap().contains("自动回滚失败"));
        assert!(!outcome.message.as_deref().unwrap().contains("已恢复"));
        assert_eq!(
            fs::read_to_string(dir.join("preserve.txt")).unwrap(),
            "backup must remain"
        );
    }
}
