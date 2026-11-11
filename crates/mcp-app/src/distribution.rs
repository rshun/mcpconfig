//! GUI 分发预览：原文和最终计划只保留在后端，确认只能消费当前一次性预览。
use crate::{App, AppError, PlanOutcome, PlanRequest};
use mcp_core::{catalog, Platform};
use mcp_io::ApplyReceipt;
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const TTL: Duration = Duration::from_secs(600);
const MAX_BYTES: u64 = 4 * 1024 * 1024;
fn stale(reason: &str) -> AppError {
    AppError::Other(format!("{reason}，本次未写入，请重新预览"))
}

// 不实现 Debug / Serialize；快照可能包含凭据，禁止进入 IPC、日志或磁盘草稿。
#[derive(PartialEq)]
struct FileSnapshot {
    path: PathBuf,
    key: String,
    text: Option<String>,
}
impl FileSnapshot {
    fn capture(path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            path: path.to_path_buf(),
            key: file_key(path)?,
            text: read(path)?,
        })
    }
    fn verify(&self) -> Result<(), AppError> {
        if self != &Self::capture(&self.path)? {
            return Err(stale("配置文件内容或实际路径已变化"));
        }
        Ok(())
    }
}
#[derive(PartialEq)]
struct Context {
    home: String,
    appdata: Option<String>,
    project: Option<String>,
    platform: Platform,
    data: String,
    backups: String,
    settings: FileSnapshot,
    custom_adapters: FileSnapshot,
    managed_state: FileSnapshot,
}
impl Context {
    fn capture(app: &App) -> Result<Self, AppError> {
        Ok(Self {
            home: file_key(&app.env.home)?,
            appdata: app.env.appdata.as_deref().map(file_key).transpose()?,
            project: app.env.project_root.as_deref().map(file_key).transpose()?,
            platform: app.platform,
            data: file_key(app.engine.home())?,
            backups: file_key(&app.engine.backups_dir())?,
            settings: FileSnapshot::capture(&app.settings_path())?,
            custom_adapters: FileSnapshot::capture(&app.custom_adapters_path())?,
            managed_state: FileSnapshot::capture(&app.engine.managed_state_path())?,
        })
    }
}
type Resolutions = BTreeMap<String, Option<(PathBuf, String)>>;
fn resolutions(app: &App, req: &PlanRequest) -> Result<Resolutions, AppError> {
    let adapter_set = app.adapter_set()?;
    let clients = req.clients.clone().unwrap_or_else(|| match &req.selection {
        Some(selection) => selection.keys().cloned().collect(),
        None => adapter_set.ids(),
    });
    let mut result = BTreeMap::new();
    for id in clients {
        if req
            .selection
            .as_ref()
            .is_some_and(|selection| selection.get(&id).is_none_or(Vec::is_empty))
        {
            continue;
        }
        let adapter = adapter_set.by_id(&id).ok_or_else(|| stale("分发目标无效"))?;
        let path = if app.check_scope_block(adapter, req.scope).is_some() {
            None
        } else {
            app.target_path(adapter, req.scope)
        };
        result.insert(
            id,
            path.map(|path| {
                let key = file_key(&path)?;
                Ok::<_, AppError>((path, key))
            })
            .transpose()?,
        );
    }
    Ok(result)
}
struct Prepared {
    id: String,
    request: PlanRequest,
    catalog: FileSnapshot,
    context: Context,
    resolutions: Resolutions,
    targets: BTreeMap<String, FileSnapshot>,
    outcome: PlanOutcome,
    created: Instant,
}
impl Prepared {
    fn verify(&self, app: &App, req: &PlanRequest) -> Result<(), AppError> {
        if self.created.elapsed() >= TTL {
            return Err(stale("分发预览已过期"));
        }
        if &self.request != req || self.context != Context::capture(app)? {
            return Err(stale("分发选择、范围、策略或软件设置已变化"));
        }
        self.catalog.verify()?;
        if self.resolutions != resolutions(app, req)? {
            return Err(stale("目标候选路径已变化"));
        }
        for snapshot in self.targets.values() {
            snapshot.verify()?;
        }
        Ok(())
    }
}

/// 单个桌面窗口只保留一个预览。新预览（即使失败）使旧预览失效。
#[derive(Default)]
pub struct DistributionSessions {
    serial: u64,
    active: Option<Prepared>,
}
impl DistributionSessions {
    pub fn invalidate(&mut self) {
        self.active = None;
    }
    pub fn cancel(&mut self, id: &str) {
        if self.active.as_ref().is_some_and(|preview| preview.id == id) {
            self.invalidate();
        }
    }
    pub fn preview(
        &mut self,
        app: &App,
        request: &PlanRequest,
    ) -> Result<(String, PlanOutcome), AppError> {
        self.invalidate();
        if !request.catalog_path.is_absolute() {
            return Err(stale("配置库必须使用绝对路径"));
        }
        let created = Instant::now();
        let source = FileSnapshot::capture(&request.catalog_path)?;
        let text = source
            .text
            .as_deref()
            .ok_or_else(|| stale("配置库不存在"))?;
        let (catalog, issues) = catalog::load(text)?;
        let context = Context::capture(app)?;
        let paths = resolutions(app, request)?;
        let mut targets = BTreeMap::new();
        for (path, key) in paths.values().flatten() {
            if !targets.contains_key(key) {
                targets.insert(key.clone(), FileSnapshot::capture(path)?);
            }
        }
        let outcome = app.plan_catalog(
            request,
            catalog,
            issues.iter().map(ToString::to_string).collect(),
        )?;
        // 计划中的基线必须正是已拍摄的目标原文，包括空文件与不存在文件的区别。
        for plan in &outcome.plans {
            let snapshot = targets
                .get(&file_key(Path::new(&plan.path))?)
                .ok_or_else(|| stale("目标路径已变化"))?;
            if snapshot.text.as_deref()
                != if plan.existed {
                    Some(plan.old_content.as_str())
                } else {
                    None
                }
            {
                return Err(stale("生成预览期间目标配置已变化"));
            }
        }
        self.serial += 1;
        let id = format!("distribution-{}-{}", std::process::id(), self.serial);
        let prepared = Prepared {
            id: id.clone(),
            request: request.clone(),
            catalog: source,
            context,
            resolutions: paths,
            targets,
            outcome,
            created,
        };
        prepared.verify(app, request)?;
        let result = prepared.outcome.clone();
        self.active = Some(prepared);
        Ok((id, result))
    }
    pub fn commit(
        &mut self,
        app: &App,
        request: &PlanRequest,
        id: &str,
        dry_run: bool,
    ) -> Result<ApplyReceipt, AppError> {
        if !self.active.as_ref().is_some_and(|preview| preview.id == id) {
            return Err(stale("分发预览无效、已取消或已使用"));
        }
        // 校验失败、演练或写入失败也消费令牌，不能重试同一个预览。
        let prepared = self.active.take().expect("matched active preview");
        prepared.verify(app, request)?;
        let text = prepared
            .catalog
            .text
            .as_deref()
            .ok_or_else(|| stale("配置库不存在"))?;
        let (catalog, issues) = catalog::load(text).map_err(|_| stale("配置库不可解析"))?;
        let current = app
            .plan_catalog(
                request,
                catalog,
                issues.iter().map(ToString::to_string).collect(),
            )
            .map_err(|_| stale("无法复核分发配置"))?;
        if current != prepared.outcome {
            return Err(stale("变量值、目标内容或最终写入计划已变化"));
        }
        // 再次检查复核期间的变化；真正写盘仍使用原预览计划，IO 层继续逐文件 CAS。
        prepared.verify(app, request)?;
        app.apply(&prepared.outcome.plans, dry_run)
    }
}
fn file_key(path: &Path) -> Result<String, AppError> {
    mcp_io::file_key(path).map_err(|_| stale("无法复核配置实际路径"))
}
fn read(path: &Path) -> Result<Option<String>, AppError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(stale("无法读取分发文件，请检查路径与权限")),
    };
    let mut text = String::new();
    file.take(MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| stale("分发文件编码无效或无法读取"))?;
    if text.len() as u64 > MAX_BYTES {
        return Err(stale("分发文件超过 4 MiB"));
    }
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_preview_is_consumed_without_creating_a_backup_or_target() {
        let root = std::env::temp_dir().join(format!(
            "mcphub-distribution-expiry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let app = App::with_home(
            root.join("data"),
            crate::HostEnv {
                home: root.clone(),
                appdata: None,
                project_root: None,
            },
            Platform::Linux,
        );
        let mut req = PlanRequest::new(root.join("catalog.yaml"));
        req.selection = Some(BTreeMap::from([(
            "claude-code".into(),
            vec!["alpha".into()],
        )]));
        std::fs::write(
            &req.catalog_path,
            "version: 1\nentries: [{name: alpha, transport: stdio, command: python}]\n",
        )
        .unwrap();
        let mut sessions = DistributionSessions::default();
        let (id, _) = sessions.preview(&app, &req).unwrap();
        sessions.active.as_mut().unwrap().created = Instant::now() - TTL;
        let error = sessions
            .commit(&app, &req, &id, false)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("过期") && error.contains("重新预览"));
        assert!(sessions.active.is_none());
        assert!(!app.engine.home().exists());
        assert!(!root.join(".claude.json").exists());
    }
}
