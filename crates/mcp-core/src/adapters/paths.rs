//! 各客户端的候选路径模板（`~` / `%APPDATA%` / `<proj>` 由上层展开）。
//!
//! 现实中文档与实际路径常有出入（Grok 双格式、Windsurf 双路径、Trae 国际/国内版），
//! 因此每个客户端按**有序候选列表**探测，取第一个存在者。

use crate::model::{Platform, Scope};

fn pick(platform: Platform, win: &str, linux: &str, mac: &str) -> String {
    match platform {
        Platform::Windows => win.to_string(),
        Platform::Linux => linux.to_string(),
        Platform::Macos => mac.to_string(),
    }
}

/// 返回该客户端在某作用域下的候选路径（按优先级）。
pub fn candidates(client_id: &str, scope: Scope, platform: Platform) -> Vec<String> {
    match (client_id, scope) {
        // ---- Claude Code：三层作用域 ----
        ("claude-code", Scope::User) => vec![pick(
            platform,
            "%USERPROFILE%/.claude.json",
            "~/.claude.json",
            "~/.claude.json",
        )],
        ("claude-code", Scope::Project) => vec![pick(
            platform,
            "<proj>/.mcp.json",
            "<proj>/.mcp.json",
            "<proj>/.mcp.json",
        )],
        ("claude-code", Scope::Local) => vec![pick(
            platform,
            "%USERPROFILE%/.claude.json#projects.<proj>",
            "~/.claude.json#projects.<proj>",
            "~/.claude.json#projects.<proj>",
        )],

        // ---- Codex ----
        ("codex", Scope::User) => vec![pick(
            platform,
            "%USERPROFILE%/.codex/config.toml",
            "~/.codex/config.toml",
            "~/.codex/config.toml",
        )],
        ("codex", Scope::Project) => vec![pick(
            platform,
            "<proj>/.codex/config.toml",
            "<proj>/.codex/config.toml",
            "<proj>/.codex/config.toml",
        )],

        // ---- WorkBuddy ----
        ("workbuddy", Scope::User) => vec![pick(
            platform,
            "%USERPROFILE%/.workbuddy/mcp.json",
            "~/.workbuddy/mcp.json",
            "~/.workbuddy/mcp.json",
        )],

        // ---- Kimi Code ----
        ("kimi-code", Scope::User) => vec![pick(
            platform,
            "%USERPROFILE%/.kimi-code/mcp.json",
            "~/.kimi-code/mcp.json",
            "~/.kimi-code/mcp.json",
        )],

        // ---- OpenCode（v2）：多候选文件名 ----
        ("opencode", Scope::User) => vec![
            pick(
                platform,
                "%USERPROFILE%/.config/opencode/opencode.json",
                "~/.config/opencode/opencode.json",
                "~/.config/opencode/opencode.json",
            ),
            pick(
                platform,
                "%USERPROFILE%/.config/opencode/opencode.jsonc",
                "~/.config/opencode/opencode.jsonc",
                "~/.config/opencode/opencode.jsonc",
            ),
            pick(
                platform,
                "%USERPROFILE%/.config/opencode/config.json",
                "~/.config/opencode/config.json",
                "~/.config/opencode/config.json",
            ),
        ],
        ("opencode", Scope::Project) => vec![
            pick(
                platform,
                "<proj>/opencode.json",
                "<proj>/opencode.json",
                "<proj>/opencode.json",
            ),
            pick(
                platform,
                "<proj>/opencode.jsonc",
                "<proj>/opencode.jsonc",
                "<proj>/opencode.jsonc",
            ),
        ],

        // ---- VS Code：用户级与便携格式路径不同 ----
        ("vscode", Scope::User) => vec![
            pick(
                platform,
                "%APPDATA%/Code/User/mcp.json",
                "~/.config/Code/User/mcp.json",
                "~/Library/Application Support/Code/User/mcp.json",
            ),
            pick(
                platform,
                "%USERPROFILE%/.copilot/mcp-config.json",
                "~/.copilot/mcp-config.json",
                "~/.copilot/mcp-config.json",
            ),
        ],
        ("vscode", Scope::Project) => vec![
            pick(
                platform,
                "<proj>/.vscode/mcp.json",
                "<proj>/.vscode/mcp.json",
                "<proj>/.vscode/mcp.json",
            ),
            pick(
                platform,
                "<proj>/.mcp.json",
                "<proj>/.mcp.json",
                "<proj>/.mcp.json",
            ),
        ],

        // ---- Antigravity ----
        ("antigravity", Scope::User) => vec![pick(
            platform,
            "%USERPROFILE%/.gemini/config/mcp_config.json",
            "~/.gemini/config/mcp_config.json",
            "~/.gemini/config/mcp_config.json",
        )],
        ("antigravity", Scope::Project) => vec![pick(
            platform,
            "<proj>/.agents/mcp_config.json",
            "<proj>/.agents/mcp_config.json",
            "<proj>/.agents/mcp_config.json",
        )],

        // ---- Grok：config.toml 优先，mcp.json 为早期形态 ----
        ("grok", Scope::User) => vec![
            pick(
                platform,
                "%USERPROFILE%/.grok/config.toml",
                "~/.grok/config.toml",
                "~/.grok/config.toml",
            ),
            pick(
                platform,
                "%USERPROFILE%/.grok/mcp.json",
                "~/.grok/mcp.json",
                "~/.grok/mcp.json",
            ),
        ],
        ("grok", Scope::Project) => vec![pick(
            platform,
            "<proj>/.grok/config.toml",
            "<proj>/.grok/config.toml",
            "<proj>/.grok/config.toml",
        )],

        // ---- Cursor ----
        ("cursor", Scope::User) => vec![pick(
            platform,
            "%USERPROFILE%/.cursor/mcp.json",
            "~/.cursor/mcp.json",
            "~/.cursor/mcp.json",
        )],

        // ---- Cline ----
        ("cline", Scope::User) => vec![pick(
            platform,
            "%USERPROFILE%/.cline/data/settings/cline_mcp_settings.json",
            "~/.cline/data/settings/cline_mcp_settings.json",
            "~/.cline/data/settings/cline_mcp_settings.json",
        )],

        // ---- Gemini CLI ----
        ("gemini-cli", Scope::User) => vec![pick(
            platform,
            "%USERPROFILE%/.gemini/settings.json",
            "~/.gemini/settings.json",
            "~/.gemini/settings.json",
        )],

        // ---- Claude Desktop ----
        ("claude-desktop", Scope::User) => vec![pick(
            platform,
            "%APPDATA%/Claude/claude_desktop_config.json",
            "~/.config/Claude/claude_desktop_config.json",
            "~/Library/Application Support/Claude/claude_desktop_config.json",
        )],

        // ---- Windsurf：两个候选路径 ----
        ("windsurf", Scope::User) => vec![
            pick(
                platform,
                "%USERPROFILE%/.codeium/windsurf/mcp_config.json",
                "~/.codeium/windsurf/mcp_config.json",
                "~/.codeium/windsurf/mcp_config.json",
            ),
            pick(
                platform,
                "%USERPROFILE%/.codeium/mcp_config.json",
                "~/.codeium/mcp_config.json",
                "~/.codeium/mcp_config.json",
            ),
        ],

        // ---- Trae：国际版 / 国内版 / 项目级 ----
        ("trae", Scope::User) => vec![
            pick(
                platform,
                "%APPDATA%/Trae/User/settings/mcp.json",
                "~/.config/Trae/User/settings/mcp.json",
                "~/Library/Application Support/Trae/User/settings/mcp.json",
            ),
            pick(
                platform,
                "%APPDATA%/Trae CN/User/mcp.json",
                "~/.config/Trae CN/User/mcp.json",
                "~/Library/Application Support/Trae CN/User/mcp.json",
            ),
            pick(
                platform,
                "%USERPROFILE%/.trae/mcp.json",
                "~/.trae/mcp.json",
                "~/.trae/mcp.json",
            ),
        ],
        ("trae", Scope::Project) => vec![pick(
            platform,
            "<proj>/.trae/mcp.json",
            "<proj>/.trae/mcp.json",
            "<proj>/.trae/mcp.json",
        )],

        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_client_has_user_path() {
        for a in super::super::clients::all() {
            let c = candidates(a.id(), Scope::User, Platform::Windows);
            assert!(!c.is_empty(), "{} 缺少用户级候选路径", a.id());
        }
    }

    #[test]
    fn trae_has_both_editions() {
        let c = candidates("trae", Scope::User, Platform::Windows);
        assert!(c.iter().any(|p| p.contains("Trae CN")), "应包含国内版候选");
        assert!(c.iter().any(|p| p.contains("Trae/")), "应包含国际版候选");
    }

    #[test]
    fn scopeless_clients_return_empty() {
        // Kimi 不支持项目级
        assert!(candidates("kimi-code", Scope::Project, Platform::Windows).is_empty());
    }
}
