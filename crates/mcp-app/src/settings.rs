//! 桌面偏好持久化与显式项目上下文。CLI 的默认行为保持不变。
use crate::{App, AppError};
use mcp_core::Scope;
use mcp_io::OutcomeStatus;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuiSettings {
    pub version: u32,
    pub catalog_path: String,
    pub backup_dir: String,
    pub default_scope: Scope,
    pub default_mode: String,
    pub last_project_dir: Option<String>,
}

impl App {
    pub fn settings_path(&self) -> PathBuf {
        self.engine.home().join("settings.json")
    }

    pub fn default_settings(&self) -> GuiSettings {
        GuiSettings {
            version: 1,
            catalog_path: self
                .engine
                .home()
                .join("catalog.yaml")
                .display()
                .to_string(),
            backup_dir: self.engine.home().join("backups").display().to_string(),
            default_scope: Scope::User,
            default_mode: "merge".into(),
            last_project_dir: None,
        }
    }

    pub fn load_settings(&self) -> Result<GuiSettings, AppError> {
        let Some(text) = self.engine.read_text(&self.settings_path())? else {
            return Ok(self.default_settings());
        };
        let settings: GuiSettings = serde_json::from_str(&text).map_err(|_| {
            AppError::Other(
                "软件设置损坏或格式不支持，请在全局设置中核对后保存；原文件未覆盖".into(),
            )
        })?;
        validate(&settings)?;
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &GuiSettings) -> Result<Vec<String>, AppError> {
        validate(settings)?;
        let text = serde_json::to_string_pretty(settings)
            .map_err(|_| AppError::Other("设置序列化失败".into()))?;
        let receipt = self.engine.save_settings(&self.settings_path(), &text)?;
        let mut warnings = Vec::new();
        for outcome in receipt.outcomes {
            match outcome.status {
                OutcomeStatus::Failed | OutcomeStatus::RollbackFailed => {
                    return Err(AppError::Other(format!(
                        "{}；备份 ID：{}",
                        outcome.message.unwrap_or_else(|| "设置保存失败".into()),
                        receipt.backup_id.as_deref().unwrap_or("无")
                    )))
                }
                OutcomeStatus::AuditFailed => warnings.push(
                    outcome
                        .message
                        .unwrap_or_else(|| "设置已保存，但审计失败".into()),
                ),
                _ => {}
            }
        }
        Ok(warnings)
    }

    /// GUI 项目配置绝不回退到进程工作目录。全局配置不继承上次项目。
    pub fn gui_context(
        mut self,
        scope: Scope,
        project_root: Option<PathBuf>,
    ) -> Result<Self, AppError> {
        let settings = self.load_settings()?;
        self.engine = self
            .engine
            .with_backups_dir(PathBuf::from(settings.backup_dir));
        self.env.project_root = if scope == Scope::User {
            None
        } else {
            let path = project_root.ok_or_else(|| AppError::Other("请先选择项目目录".into()))?;
            if !path.is_absolute() || !path.is_dir() {
                return Err(AppError::Other("项目目录必须是存在的绝对目录".into()));
            }
            Some(
                std::fs::canonicalize(path)
                    .map_err(|_| AppError::Other("项目目录不可读取".into()))?,
            )
        };
        Ok(self)
    }

    pub fn create_catalog(&self, path: &Path) -> Result<Vec<String>, AppError> {
        if !path.is_absolute() {
            return Err(AppError::Other("配置库必须使用绝对路径".into()));
        }
        if path
            .try_exists()
            .map_err(|_| AppError::Other("无法检查配置库路径".into()))?
        {
            return Err(AppError::Other(
                "文件已经存在，请加载现有配置库或选择新路径".into(),
            ));
        }
        let catalog = mcp_core::catalog::parse("version: 1\nentries: []\n")?;
        self.save_catalog(path, &catalog)
    }
}

fn validate(settings: &GuiSettings) -> Result<(), AppError> {
    if settings.version != 1
        || !matches!(settings.default_scope, Scope::User | Scope::Project)
        || !["merge", "replace"].contains(&settings.default_mode.as_str())
    {
        return Err(AppError::Other("设置版本、默认范围或写入策略无效".into()));
    }
    let catalog = Path::new(&settings.catalog_path);
    let backups = Path::new(&settings.backup_dir);
    if !catalog.is_absolute() || !backups.is_absolute() {
        return Err(AppError::Other("配置库位置和备份目录必须是绝对路径".into()));
    }
    if catalog.is_dir() || (backups.exists() && !backups.is_dir()) {
        return Err(AppError::Other(
            "配置库需要文件路径，备份位置需要目录路径".into(),
        ));
    }
    if let Some(project) = &settings.last_project_dir {
        if !Path::new(project).is_absolute() {
            return Err(AppError::Other("项目目录必须是绝对路径".into()));
        }
    }
    Ok(())
}
