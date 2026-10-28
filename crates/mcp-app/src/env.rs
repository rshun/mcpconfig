//! 宿主环境：路径模板展开。

use mcp_core::model::Platform;
use std::path::{Path, PathBuf};

/// 运行宿主的关键目录。
#[derive(Debug, Clone, Default)]
pub struct HostEnv {
    pub home: PathBuf,
    /// Windows 的 `%APPDATA%`；其他平台为 `~/.config`
    pub appdata: Option<PathBuf>,
    /// 项目根目录（用于 `<proj>` 与 `${PROJECT_ROOT}`）
    pub project_root: Option<PathBuf>,
}

impl HostEnv {
    /// 从当前进程环境读取。
    pub fn from_process(project_root: Option<PathBuf>) -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));

        let appdata = std::env::var("APPDATA")
            .ok()
            .map(PathBuf::from)
            .or_else(|| Some(home.join(".config")));

        Self {
            home,
            appdata,
            project_root: project_root.or_else(|| std::env::current_dir().ok()),
        }
    }

    /// 展开路径模板：`~` / `%USERPROFILE%` / `%APPDATA%` / `<proj>`。
    ///
    /// 返回 `None` 表示模板依赖的变量缺失（例如没有 `%APPDATA%`）。
    pub fn expand(&self, template: &str, _platform: Platform) -> Option<PathBuf> {
        let home = self.home.to_string_lossy().replace('\\', "/");
        let mut s = template.replace('\\', "/");

        if s == "~" {
            s = home.clone();
        } else if let Some(rest) = s.strip_prefix("~/") {
            s = format!("{home}/{rest}");
        }

        if s.contains("%USERPROFILE%") {
            s = s.replace("%USERPROFILE%", &home);
        }

        if s.contains("%APPDATA%") {
            let appdata = self.appdata.as_ref()?.to_string_lossy().replace('\\', "/");
            s = s.replace("%APPDATA%", &appdata);
        }

        if s.contains("<proj>") {
            let proj = self
                .project_root
                .as_ref()?
                .to_string_lossy()
                .replace('\\', "/");
            s = s.replace("<proj>", &proj);
        }

        // Windows canonicalize 产生 \\?\ 前缀，该路径模式只接受反斜线。
        // 模板替换时的统一分隔符不能留在最终的 verbatim 路径中。
        #[cfg(windows)]
        if s.starts_with("//?/") {
            s = s.replace('/', "\\");
        }
        Some(PathBuf::from(s))
    }

    /// 依次展开候选模板，返回全部可展开的结果（保持顺序）。
    pub fn expand_all(&self, templates: &[String], platform: Platform) -> Vec<PathBuf> {
        templates
            .iter()
            .filter_map(|t| self.expand(t, platform))
            .collect()
    }
}

/// 判断路径是否存在（文件或目录）。
pub fn exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> HostEnv {
        HostEnv {
            home: PathBuf::from("C:/Users/demo"),
            appdata: Some(PathBuf::from("C:/Users/demo/AppData/Roaming")),
            project_root: Some(PathBuf::from("C:/work/proj")),
        }
    }

    #[test]
    fn expands_home() {
        let p = env().expand("~/.claude.json", Platform::Windows).unwrap();
        assert_eq!(
            p.to_string_lossy().replace('\\', "/"),
            "C:/Users/demo/.claude.json"
        );
    }

    #[test]
    fn expands_userprofile_and_appdata() {
        let e = env();
        let a = e
            .expand("%USERPROFILE%/.codex/config.toml", Platform::Windows)
            .unwrap();
        assert!(a.to_string_lossy().contains("codex"));
        let b = e
            .expand("%APPDATA%/Code/User/mcp.json", Platform::Windows)
            .unwrap();
        assert!(b.to_string_lossy().contains("Roaming"));
    }

    #[test]
    fn expands_project() {
        let p = env().expand("<proj>/.mcp.json", Platform::Windows).unwrap();
        assert_eq!(
            p.to_string_lossy().replace('\\', "/"),
            "C:/work/proj/.mcp.json"
        );
    }

    #[test]
    fn missing_appdata_returns_none() {
        let e = HostEnv {
            home: PathBuf::from("/home/dev"),
            appdata: None,
            project_root: None,
        };
        assert!(e.expand("%APPDATA%/x/mcp.json", Platform::Linux).is_none());
    }

    #[test]
    fn missing_project_returns_none() {
        let e = HostEnv {
            home: PathBuf::from("/home/dev"),
            appdata: None,
            project_root: None,
        };
        assert!(e.expand("<proj>/.mcp.json", Platform::Linux).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_project_and_unc_paths_preserve_windows_prefixes() {
        for root in [r"\\?\C:\work\项目", r"\\?\UNC\host\share\项目"] {
            let e = HostEnv {
                project_root: Some(PathBuf::from(root)),
                ..env()
            };
            assert_eq!(
                e.expand("<proj>/.codex/config.toml", Platform::Windows)
                    .unwrap(),
                PathBuf::from(root).join(".codex").join("config.toml")
            );
        }
    }
}
