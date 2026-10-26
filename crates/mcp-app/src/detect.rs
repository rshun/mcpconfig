//! 客户端检测：三通道（配置路径 / 可执行文件 / 安装目录）+ 四态。

use crate::env::HostEnv;
use mcp_core::adapters::{self, paths, Adapter, ClientKind, Format};
use mcp_core::document::ConfigDoc;
use mcp_core::model::{Platform, Scope};
use std::path::PathBuf;

/// 四种状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectState {
    /// 客户端存在 + 配置文件在
    Ready,
    /// 客户端在，配置文件不存在（可创建）
    InstalledNoConfig,
    /// 配置文件在，未检测到客户端（可能是残留）
    ConfigOnly,
    /// 两者都没有
    NotDetected,
}

impl DetectState {
    pub fn label(self) -> &'static str {
        match self {
            DetectState::Ready => "已就绪",
            DetectState::InstalledNoConfig => "已安装未配置",
            DetectState::ConfigOnly => "仅配置存在",
            DetectState::NotDetected => "未检测",
        }
    }
}

/// 一个客户端的检测结果。
#[derive(Debug, Clone)]
pub struct ClientStatus {
    pub id: String,
    pub name: String,
    pub kind: ClientKind,
    pub format: Format,
    pub scopes: Vec<Scope>,
    /// 命中的可执行文件路径
    pub binary: Option<String>,
    /// 实际选中的配置路径（已展开）
    pub chosen_path: Option<PathBuf>,
    pub candidates: Vec<PathBuf>,
    pub path_existed: bool,
    pub state: DetectState,
    /// 该文件中已存在的条目名
    pub configured_entries: Vec<String>,
    /// 该作用域不支持时的原因
    pub scope_block: Option<String>,
}

/// 已知的 CLI 可执行文件名（用于通道 B）。
fn binary_names(id: &str) -> &'static [&'static str] {
    match id {
        "claude-code" => &["claude"],
        "codex" => &["codex"],
        "kimi-code" => &["kimi"],
        "opencode" => &["opencode"],
        "grok" => &["grok"],
        "gemini-cli" => &["gemini"],
        "vscode" => &["code"],
        "cursor" => &["cursor"],
        "antigravity" => &["agy"],
        "windsurf" => &["windsurf"],
        "trae" => &["trae"],
        _ => &[],
    }
}

/// 在 PATH 中查找可执行文件。
fn which(names: &[&str], platform: Platform) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    let exts: &[&str] = if platform == Platform::Windows {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    for dir in dirs {
        for name in names {
            for ext in exts {
                let cand = dir.join(format!("{name}{ext}"));
                if cand.is_file() {
                    return Some(cand.display().to_string());
                }
            }
        }
    }
    None
}

/// 检测单个客户端。
pub fn detect_client(
    adapter: &dyn Adapter,
    env: &HostEnv,
    platform: Platform,
    scope: Scope,
) -> ClientStatus {
    let scope_block = crate::scope_block_reason(adapter.id(), scope);

    let templates = paths::candidates(adapter.id(), scope, platform);
    let candidates = env.expand_all(&templates, platform);

    // 选第一个存在的；都不存在则取第一个候选（用于新建）
    let existing = candidates.iter().find(|p| p.exists());
    let chosen_path = existing.cloned().or_else(|| candidates.first().cloned());
    let path_existed = existing.is_some();

    let binary = which(binary_names(adapter.id()), platform);

    let state = match (binary.is_some(), path_existed) {
        (true, true) => DetectState::Ready,
        (true, false) => DetectState::InstalledNoConfig,
        (false, true) => DetectState::ConfigOnly,
        (false, false) => DetectState::NotDetected,
    };

    // 读取已有条目（仅用于状态展示，不参与写入决策）
    let mut configured_entries = Vec::new();
    if let Some(path) = &chosen_path {
        if path.is_file() {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(doc) = ConfigDoc::parse(adapter.spec().format, &text) {
                    configured_entries = doc.names(&adapter.root_key(scope));
                }
            }
        }
    }

    ClientStatus {
        id: adapter.id().to_string(),
        name: adapter.display_name().to_string(),
        kind: adapter.kind(),
        format: adapter.spec().format,
        scopes: adapter.spec().scopes.to_vec(),
        binary,
        chosen_path,
        candidates,
        path_existed,
        state,
        configured_entries,
        scope_block,
    }
}

/// 检测全部 14 个客户端。
pub fn detect_all(env: &HostEnv, platform: Platform, scope: Scope) -> Vec<ClientStatus> {
    adapters::all()
        .iter()
        .map(|a| detect_client(*a, env, platform, scope))
        .collect()
}

/// 按 id 检测。
pub fn detect_one(
    id: &str,
    env: &HostEnv,
    platform: Platform,
    scope: Scope,
) -> Option<ClientStatus> {
    adapters::by_id(id).map(|a| detect_client(a, env, platform, scope))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_home(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "mcphub-detect-{}-{}-{}",
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

    #[test]
    fn missing_file_is_not_configured() {
        let home = tmp_home("miss");
        let env = HostEnv {
            home: home.clone(),
            appdata: Some(home.join("AppData/Roaming")),
            project_root: Some(home.join("proj")),
        };
        let s = detect_one("claude-code", &env, Platform::Windows, Scope::User).unwrap();
        assert!(!s.path_existed);
        assert!(s.chosen_path.is_some(), "应给出可创建的路径");
        assert!(s.configured_entries.is_empty());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn existing_file_lists_entries() {
        let home = tmp_home("exist");
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join(".claude.json"),
            r#"{"mcpServers":{"a":{},"b":{}}}"#,
        )
        .unwrap();
        let env = HostEnv {
            home: home.clone(),
            appdata: None,
            project_root: None,
        };
        let s = detect_one("claude-code", &env, Platform::Windows, Scope::User).unwrap();
        assert!(s.path_existed);
        let mut names = s.configured_entries.clone();
        names.sort();
        assert_eq!(names, vec!["a", "b"]);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn broken_file_yields_no_entries_but_no_panic() {
        let home = tmp_home("broken");
        fs::write(home.join(".claude.json"), "{ broken").unwrap();
        let env = HostEnv {
            home: home.clone(),
            appdata: None,
            project_root: None,
        };
        let s = detect_one("claude-code", &env, Platform::Windows, Scope::User).unwrap();
        assert!(s.path_existed);
        assert!(s.configured_entries.is_empty());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn detects_all_14() {
        let home = tmp_home("all");
        let env = HostEnv {
            home: home.clone(),
            appdata: Some(home.join("AppData/Roaming")),
            project_root: Some(home.join("proj")),
        };
        let all = detect_all(&env, Platform::Windows, Scope::User);
        assert_eq!(all.len(), 14);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn local_scope_blocked_at_app_level() {
        let home = tmp_home("local");
        let env = HostEnv {
            home: home.clone(),
            appdata: None,
            project_root: None,
        };
        let s = detect_one("claude-code", &env, Platform::Windows, Scope::Local).unwrap();
        assert!(s.scope_block.is_some());
        let _ = fs::remove_dir_all(&home);
    }
}
