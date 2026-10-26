//! 规范化中间模型（IR）。
//!
//! 与任何客户端无关；适配器只负责 IR ↔ 客户端格式的双向转换。

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;

/// 目标操作系统。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Linux,
    Macos,
}

impl Platform {
    /// 当前运行平台。
    pub fn current() -> Self {
        #[cfg(target_os = "windows")]
        {
            Platform::Windows
        }
        #[cfg(target_os = "linux")]
        {
            Platform::Linux
        }
        #[cfg(target_os = "macos")]
        {
            Platform::Macos
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        {
            Platform::Linux
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Windows => "windows",
            Platform::Linux => "linux",
            Platform::Macos => "macos",
        }
    }
}

/// 作用域。`Local` 仅 Claude Code 支持。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    User,
    Project,
    Local,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Project => "project",
            Scope::Local => "local",
        }
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Scope::User => "用户级",
            Scope::Project => "项目级",
            Scope::Local => "本地级",
        };
        write!(f, "{s}")
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 传输类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Stdio,
    Http,
    Sse,
}

/// 密钥来源：环境变量名（推荐）或明文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    EnvRef(String),
    Literal(String),
}

/// 认证方式。v0 只支持 Bearer。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    Bearer { token: SecretSource },
}

impl Auth {
    pub fn env_ref(name: impl Into<String>) -> Self {
        Auth::Bearer {
            token: SecretSource::EnvRef(name.into()),
        }
    }

    pub fn env_name(&self) -> Option<&str> {
        match self {
            Auth::Bearer {
                token: SecretSource::EnvRef(n),
            } => Some(n),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthSpec {
    #[serde(rename = "type")]
    kind: String,
    token_env: Option<String>,
    token: Option<String>,
}

impl<'de> Deserialize<'de> for Auth {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = AuthSpec::deserialize(d)?;
        let kind = s.kind.to_ascii_lowercase();
        if kind != "bearer" {
            return Err(de::Error::custom(format!("暂只支持 bearer，收到 {kind}")));
        }
        match (s.token_env, s.token) {
            (Some(name), None) if !name.trim().is_empty() => Ok(Auth::Bearer {
                token: SecretSource::EnvRef(name),
            }),
            (None, Some(v)) => Ok(Auth::Bearer {
                token: SecretSource::Literal(v),
            }),
            (Some(_), Some(_)) => Err(de::Error::custom("token_env 与 token 只能二选一")),
            _ => Err(de::Error::custom("bearer 需要 token_env（推荐）或 token")),
        }
    }
}

impl Serialize for Auth {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let Auth::Bearer { token } = self;
        let mut m = s.serialize_map(Some(3))?;
        m.serialize_entry("type", "bearer")?;
        match token {
            SecretSource::EnvRef(n) => m.serialize_entry("token_env", n)?,
            SecretSource::Literal(v) => m.serialize_entry("token", v)?,
        }
        m.end()
    }
}

fn de_scope_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Scope>, D::Error> {
    let raw = Option::<String>::deserialize(d)?;
    match raw.as_deref() {
        None | Some("") | Some("inherit") => Ok(None),
        Some("user") => Ok(Some(Scope::User)),
        Some("project") => Ok(Some(Scope::Project)),
        Some("local") => Ok(Some(Scope::Local)),
        Some(other) => Err(de::Error::custom(format!("未知作用域：{other}"))),
    }
}

/// 平台差异覆盖：只写需要覆盖的字段。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
}

/// 一条 MCP 配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpEntry {
    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    pub transport: Transport,

    // ---- stdio ----
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,

    // ---- remote ----
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,

    #[serde(default = "default_true")]
    pub enabled: bool,

    /// `None` 表示继承全局作用域。
    #[serde(
        default,
        deserialize_with = "de_scope_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub scope: Option<Scope>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<Platform, EntryOverride>,
}

fn default_true() -> bool {
    true
}

impl McpEntry {
    pub fn new_stdio(
        name: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: None,
            transport: Transport::Stdio,
            command: Some(command.into()),
            args,
            env: BTreeMap::new(),
            cwd: None,
            url: None,
            headers: BTreeMap::new(),
            auth: None,
            timeout_seconds: None,
            enabled: true,
            scope: None,
            tags: Vec::new(),
            overrides: BTreeMap::new(),
        }
    }

    pub fn new_http(name: impl Into<String>, url: impl Into<String>) -> Self {
        let mut e = Self::new_stdio(name, "", Vec::new());
        e.transport = Transport::Http;
        e.command = None;
        e.url = Some(url.into());
        e
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.transport, Transport::Http | Transport::Sse)
    }
}

/// 清单文件根结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<McpEntry>,
}

fn default_version() -> u32 {
    1
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}
