//! 宿主环境：路径模板与 `#fragment` 动态容器路径展开。

use mcp_core::model::Platform;
use std::path::{Path, PathBuf};

/// 展开后的目标文件与可选内部容器前缀（如 `#projects.<proj>` → `["projects", "<proj>"]`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedTarget {
    pub path: PathBuf,
    pub container_prefix: Vec<String>,
}

/// 运行宿主的关键目录。
#[derive(Debug, Clone, Default)]
pub struct HostEnv {
    pub home: PathBuf,
    /// Windows 的 `%APPDATA%`；其他平台为 `~/.config`
    pub appdata: Option<PathBuf>,
    /// 项目根目录（用于 `<proj>` 与 `${PROJECT_ROOT}`）
    pub project_root: Option<PathBuf>,
}

fn clean_project_key(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix("//?/UNC/") {
        format!("//{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else if let Some(rest) = raw.strip_prefix("//?/") {
        rest.to_string()
    } else {
        raw.to_string()
    }
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

    /// 展开路径模板及可选 `#fragment` 容器路径（例如 `%USERPROFILE%/.claude.json#projects.<proj>`）。
    pub fn expand_target(&self, template: &str, _platform: Platform) -> Option<ExpandedTarget> {
        let (file_tpl, frag_opt) = match template.split_once('#') {
            Some((f, frag)) => (f, Some(frag)),
            None => (template, None),
        };

        let home = self.home.to_string_lossy().replace('\\', "/");
        let mut s = file_tpl.replace('\\', "/");

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

        let mut container_prefix = Vec::new();
        if let Some(frag) = frag_opt {
            let trimmed = frag.trim();
            if trimmed.is_empty() {
                return None;
            }
            for seg_tpl in trimmed.split('.') {
                if seg_tpl.is_empty() {
                    return None;
                }
                if seg_tpl.contains("<proj>") {
                    let proj_raw = self.project_root.as_ref()?.to_string_lossy();
                    let proj_clean = clean_project_key(&proj_raw);
                    container_prefix.push(seg_tpl.replace("<proj>", &proj_clean));
                } else {
                    container_prefix.push(seg_tpl.to_string());
                }
            }
        }

        Some(ExpandedTarget {
            path: PathBuf::from(s),
            container_prefix,
        })
    }

    /// 展开路径模板：`~` / `%USERPROFILE%` / `%APPDATA%` / `<proj>`（自动剥离 `#fragment` 返回物理文件路径）。
    ///
    /// 返回 `None` 表示模板依赖的变量缺失（例如没有 `%APPDATA%` 或 `<proj>`）。
    pub fn expand(&self, template: &str, platform: Platform) -> Option<PathBuf> {
        self.expand_target(template, platform).map(|t| t.path)
    }

    /// 依次展开候选模板，返回全部可展开的物理文件路径（保持顺序）。
    pub fn expand_all(&self, templates: &[String], platform: Platform) -> Vec<PathBuf> {
        templates
            .iter()
            .filter_map(|t| self.expand(t, platform))
            .collect()
    }

    /// 依次展开候选模板，返回全部带动态容器路径的结果（保持顺序）。
    pub fn expand_all_targets(
        &self,
        templates: &[String],
        platform: Platform,
    ) -> Vec<ExpandedTarget> {
        templates
            .iter()
            .filter_map(|t| self.expand_target(t, platform))
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
            project_root: Some(PathBuf::from("C:/work/my.proj")),
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
    fn expands_fragment_preserving_dots_in_project_path() {
        let t = env()
            .expand_target("%USERPROFILE%/.claude.json#projects.<proj>", Platform::Windows)
            .unwrap();
        assert_eq!(
            t.path.to_string_lossy().replace('\\', "/"),
            "C:/Users/demo/.claude.json"
        );
        assert_eq!(
            t.container_prefix,
            vec!["projects".to_string(), "C:/work/my.proj".to_string()]
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
            "C:/work/my.proj/.mcp.json"
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
        assert!(e
            .expand("~/.claude.json#projects.<proj>", Platform::Linux)
            .is_none());
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
