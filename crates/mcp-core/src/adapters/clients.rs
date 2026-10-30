//! 14 个客户端的适配器定义。
//!
//! 全部用声明式 `AdapterSpec`；只有 VS Code 因根键随作用域变化而需要钩子。

use super::{Adapter, AdapterSpec, ClientKind, EnabledKey, EntryStyle, Format, SecretStyle};
use crate::model::Scope;

/// 通用适配器：行为完全由 spec 决定。
pub struct Plain {
    id: &'static str,
    name: &'static str,
    kind: ClientKind,
    spec: AdapterSpec,
}

impl Adapter for Plain {
    fn id(&self) -> &'static str {
        self.id
    }
    fn display_name(&self) -> &'static str {
        self.name
    }
    fn kind(&self) -> ClientKind {
        self.kind
    }
    fn spec(&self) -> &AdapterSpec {
        &self.spec
    }
}

const USER: &[Scope] = &[Scope::User];
const USER_PROJECT: &[Scope] = &[Scope::User, Scope::Project];
const ALL_SCOPES: &[Scope] = &[Scope::User, Scope::Project, Scope::Local];

// ---------------------------------------------------------------- 1. WorkBuddy

pub static WORKBUDDY: Plain = Plain {
    id: "workbuddy",
    name: "WorkBuddy",
    kind: ClientKind::Cli,
    spec: AdapterSpec {
        format: Format::Json,
        root_key: &["mcpServers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        remote_url_key: "url",
        type_field: Some("type"),
        stdio_type_value: None,
        remote_type_value: None,
        enabled_key: EnabledKey::None,
        supports_cwd: true,
        secret_style: SecretStyle::LiteralOnly,
        timeout_field: None,
        scopes: USER,
    },
};

// ---------------------------------------------------------------- 2. Claude Code

pub static CLAUDE_CODE: Plain = Plain {
    id: "claude-code",
    name: "Claude Code",
    kind: ClientKind::Cli,
    spec: AdapterSpec {
        format: Format::Json,
        root_key: &["mcpServers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        remote_url_key: "url",
        type_field: Some("type"),
        stdio_type_value: Some("stdio"),
        remote_type_value: None,
        enabled_key: EnabledKey::None,
        supports_cwd: false,
        secret_style: SecretStyle::EnvExpansion("${", "}"),
        timeout_field: None,
        scopes: ALL_SCOPES,
    },
};

// ---------------------------------------------------------------- 3. Codex CLI

pub static CODEX: Plain = Plain {
    id: "codex",
    name: "Codex CLI",
    kind: ClientKind::Cli,
    spec: AdapterSpec {
        format: Format::Toml,
        root_key: &["mcp_servers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        remote_url_key: "url",
        type_field: None,
        stdio_type_value: None,
        remote_type_value: None,
        enabled_key: EnabledKey::Enabled,
        supports_cwd: true,
        // 原生字段：bearer_token_env_var，密钥永不落盘
        secret_style: SecretStyle::NativeEnvField,
        timeout_field: Some("startup_timeout_sec"),
        scopes: USER_PROJECT,
    },
};

// ---------------------------------------------------------------- 4. Kimi Code

pub static KIMI_CODE: Plain = Plain {
    id: "kimi-code",
    name: "Kimi Code",
    kind: ClientKind::Cli,
    spec: AdapterSpec {
        format: Format::Json,
        root_key: &["mcpServers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        remote_url_key: "url",
        type_field: None,
        stdio_type_value: None,
        remote_type_value: None,
        enabled_key: EnabledKey::None,
        supports_cwd: true,
        secret_style: SecretStyle::LiteralOnly,
        timeout_field: None,
        scopes: USER,
    },
};

// ---------------------------------------------------------------- 5. OpenCode（仅 v2）

pub static OPENCODE: Plain = Plain {
    id: "opencode",
    name: "OpenCode",
    kind: ClientKind::Cli,
    spec: AdapterSpec {
        format: Format::Jsonc,
        root_key: &["mcp", "servers"],
        entry_style: EntryStyle::CommandArray,
        env_key: "environment",
        remote_url_key: "url",
        type_field: Some("type"),
        stdio_type_value: Some("local"),
        remote_type_value: Some("remote"),
        // v2 用 disabled，语义反转
        enabled_key: EnabledKey::Disabled,
        supports_cwd: true,
        secret_style: SecretStyle::EnvExpansion("{env:", "}"),
        timeout_field: None,
        scopes: USER_PROJECT,
    },
};

// ---------------------------------------------------------------- 6. VS Code

pub struct VsCode;

impl Adapter for VsCode {
    fn id(&self) -> &'static str {
        "vscode"
    }
    fn display_name(&self) -> &'static str {
        "VS Code"
    }
    fn kind(&self) -> ClientKind {
        ClientKind::Ide
    }
    fn spec(&self) -> &AdapterSpec {
        static S: AdapterSpec = AdapterSpec {
            format: Format::Json,
            root_key: &["servers"],
            entry_style: EntryStyle::CommandArgs,
            env_key: "env",
            remote_url_key: "url",
            type_field: Some("type"),
            stdio_type_value: None,
            remote_type_value: None,
            enabled_key: EnabledKey::None,
            supports_cwd: false,
            secret_style: SecretStyle::LiteralOnly,
            timeout_field: None,
            scopes: USER_PROJECT,
        };
        &S
    }
    /// 用户级用 VS Code 专有格式 `servers`，项目级/便携格式用 `mcpServers`。
    fn root_key(&self, scope: Scope) -> Vec<String> {
        match scope {
            Scope::User => vec!["servers".to_string()],
            _ => vec!["mcpServers".to_string()],
        }
    }

    fn root_key_for_path(&self, _scope: Scope, path: &std::path::Path) -> Vec<String> {
        let portable = path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| {
                name.eq_ignore_ascii_case(".mcp.json")
                    || name.eq_ignore_ascii_case("mcp-config.json")
            });
        vec![if portable { "mcpServers" } else { "servers" }.to_string()]
    }

    fn format_for_path(&self, path: &std::path::Path) -> Format {
        if path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("mcp.json"))
        {
            Format::Jsonc
        } else {
            Format::Json
        }
    }
}

// ---------------------------------------------------------------- 7. Antigravity

pub static ANTIGRAVITY: Plain = Plain {
    id: "antigravity",
    name: "Antigravity",
    kind: ClientKind::Ide,
    spec: AdapterSpec {
        format: Format::Json,
        root_key: &["mcpServers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        // 远程必须用 serverUrl，不支持 url / httpUrl
        remote_url_key: "serverUrl",
        type_field: None,
        stdio_type_value: None,
        remote_type_value: None,
        enabled_key: EnabledKey::None,
        supports_cwd: false,
        secret_style: SecretStyle::LiteralOnly,
        timeout_field: Some("timeoutSeconds"),
        scopes: USER_PROJECT,
    },
};

// ---------------------------------------------------------------- 8. Grok

pub static GROK: Plain = Plain {
    id: "grok",
    name: "Grok",
    kind: ClientKind::Cli,
    spec: AdapterSpec {
        format: Format::Toml,
        root_key: &["mcp_servers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        remote_url_key: "url",
        type_field: None,
        stdio_type_value: None,
        remote_type_value: None,
        enabled_key: EnabledKey::Enabled,
        supports_cwd: false,
        secret_style: SecretStyle::EnvExpansion("${", "}"),
        timeout_field: Some("startup_timeout_sec"),
        scopes: USER_PROJECT,
    },
};

// ---------------------------------------------------------------- 9. Cursor

pub static CURSOR: Plain = Plain {
    id: "cursor",
    name: "Cursor",
    kind: ClientKind::Ide,
    spec: AdapterSpec {
        format: Format::Json,
        root_key: &["mcpServers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        remote_url_key: "url",
        type_field: None,
        stdio_type_value: None,
        remote_type_value: None,
        enabled_key: EnabledKey::None,
        supports_cwd: false,
        secret_style: SecretStyle::LiteralOnly,
        timeout_field: None,
        scopes: USER,
    },
};

// ---------------------------------------------------------------- 10. Cline

pub static CLINE: Plain = Plain {
    id: "cline",
    name: "Cline",
    kind: ClientKind::Ide,
    spec: AdapterSpec {
        format: Format::Json,
        root_key: &["mcpServers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        remote_url_key: "url",
        type_field: None,
        stdio_type_value: None,
        remote_type_value: None,
        enabled_key: EnabledKey::None,
        supports_cwd: false,
        secret_style: SecretStyle::LiteralOnly,
        timeout_field: None,
        scopes: USER,
    },
};

// ---------------------------------------------------------------- 11. Gemini CLI

pub static GEMINI_CLI: Plain = Plain {
    id: "gemini-cli",
    name: "Gemini CLI",
    kind: ClientKind::Cli,
    spec: AdapterSpec {
        format: Format::Json,
        root_key: &["mcpServers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        remote_url_key: "url",
        type_field: None,
        stdio_type_value: None,
        remote_type_value: None,
        enabled_key: EnabledKey::None,
        supports_cwd: false,
        secret_style: SecretStyle::LiteralOnly,
        timeout_field: None,
        scopes: USER,
    },
};

// ---------------------------------------------------------------- 12. Claude Desktop

pub static CLAUDE_DESKTOP: Plain = Plain {
    id: "claude-desktop",
    name: "Claude Desktop",
    kind: ClientKind::Desktop,
    spec: AdapterSpec {
        format: Format::Json,
        root_key: &["mcpServers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        remote_url_key: "url",
        type_field: None,
        stdio_type_value: None,
        remote_type_value: None,
        enabled_key: EnabledKey::None,
        supports_cwd: false,
        secret_style: SecretStyle::LiteralOnly,
        timeout_field: None,
        scopes: USER,
    },
};

// ---------------------------------------------------------------- 13. Windsurf

pub static WINDSURF: Plain = Plain {
    id: "windsurf",
    name: "Windsurf",
    kind: ClientKind::Ide,
    spec: AdapterSpec {
        format: Format::Json,
        root_key: &["mcpServers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        remote_url_key: "serverUrl",
        type_field: None,
        stdio_type_value: None,
        remote_type_value: None,
        enabled_key: EnabledKey::None,
        supports_cwd: false,
        // 官方支持 ${env:VAR} 在 headers / url 中插值
        secret_style: SecretStyle::EnvExpansion("${env:", "}"),
        timeout_field: None,
        scopes: USER,
    },
};

// ---------------------------------------------------------------- 14. Trae

pub static TRAE: Plain = Plain {
    id: "trae",
    name: "Trae",
    kind: ClientKind::Ide,
    spec: AdapterSpec {
        format: Format::Json,
        root_key: &["mcpServers"],
        entry_style: EntryStyle::CommandArgs,
        env_key: "env",
        remote_url_key: "url",
        type_field: None,
        stdio_type_value: None,
        remote_type_value: None,
        enabled_key: EnabledKey::None,
        supports_cwd: true,
        secret_style: SecretStyle::LiteralOnly,
        timeout_field: None,
        scopes: USER_PROJECT,
    },
};

// ---------------------------------------------------------------- 注册表

static ALL: &[&dyn Adapter] = &[
    &CLAUDE_CODE,
    &CODEX,
    &WORKBUDDY,
    &KIMI_CODE,
    &OPENCODE,
    &VsCode,
    &ANTIGRAVITY,
    &GROK,
    &CURSOR,
    &CLINE,
    &GEMINI_CLI,
    &CLAUDE_DESKTOP,
    &WINDSURF,
    &TRAE,
];

/// 全部适配器（14 个，顺序即 UI 列顺序）。
pub fn all() -> &'static [&'static dyn Adapter] {
    ALL
}

/// 按 id 查找。
pub fn by_id(id: &str) -> Option<&'static dyn Adapter> {
    ALL.iter().copied().find(|a| a.id() == id)
}

/// 按 id 列表查找；未知 id 报错。
pub fn by_ids(ids: &[String]) -> crate::Result<Vec<&'static dyn Adapter>> {
    ids.iter()
        .map(|id| by_id(id).ok_or_else(|| crate::Error::UnknownClient(id.clone())))
        .collect()
}
