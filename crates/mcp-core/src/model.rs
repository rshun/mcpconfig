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

/// 强类型 stdio 传输视图（消除可选字段非法组合）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdioTransport {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub timeout_seconds: Option<u64>,
}

/// 远程传输协议子类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteKind {
    Http,
    Sse,
}

/// 强类型远程传输视图（消除可选字段非法组合）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTransport {
    pub kind: RemoteKind,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub auth: Option<Auth>,
    pub timeout_seconds: Option<u64>,
}

/// 校验合一的传输规格代数类型（Sum Type）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportSpec {
    Stdio(StdioTransport),
    Remote(RemoteTransport),
}

/// 密钥来源：环境变量名（推荐）或明文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    EnvRef(String),
    Literal(String),
}

/// 认证方式：支持标准 Bearer 与自定义请求头（Header / API Key）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    Bearer {
        token: SecretSource,
    },
    Header {
        header: String,
        prefix: Option<String>,
        value: SecretSource,
    },
}

impl Auth {
    pub fn env_ref(name: impl Into<String>) -> Self {
        Auth::Bearer {
            token: SecretSource::EnvRef(name.into()),
        }
    }

    pub fn header_env_ref(
        header: impl Into<String>,
        prefix: Option<String>,
        name: impl Into<String>,
    ) -> Self {
        Auth::Header {
            header: header.into(),
            prefix,
            value: SecretSource::EnvRef(name.into()),
        }
    }

    pub fn env_name(&self) -> Option<&str> {
        match self {
            Auth::Bearer {
                token: SecretSource::EnvRef(n),
            }
            | Auth::Header {
                value: SecretSource::EnvRef(n),
                ..
            } => Some(n),
            _ => None,
        }
    }

    pub fn is_literal(&self) -> bool {
        matches!(
            self,
            Auth::Bearer {
                token: SecretSource::Literal(_),
            } | Auth::Header {
                value: SecretSource::Literal(_),
                ..
            }
        )
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthSpec {
    #[serde(rename = "type")]
    kind: String,
    header: Option<String>,
    prefix: Option<String>,
    token_env: Option<String>,
    token: Option<String>,
    value_env: Option<String>,
    value: Option<String>,
}

impl<'de> Deserialize<'de> for Auth {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = AuthSpec::deserialize(d)?;
        let kind = s.kind.to_ascii_lowercase();
        let env_opt = match (s.token_env, s.value_env) {
            (Some(_), Some(_)) => return Err(de::Error::custom("token_env 与 value_env 只能二选一")),
            (a, b) => a.or(b),
        };
        let lit_opt = match (s.token, s.value) {
            (Some(_), Some(_)) => return Err(de::Error::custom("token 与 value 只能二选一")),
            (a, b) => a.or(b),
        };
        match kind.as_str() {
            "bearer" => {
                if s.header.is_some() || s.prefix.is_some() {
                    return Err(de::Error::custom("bearer 不接受 header 或 prefix 字段"));
                }
                match (env_opt, lit_opt) {
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
            "header" | "api_key" => {
                let header = s
                    .header
                    .map(|h| h.trim().to_string())
                    .filter(|h| !h.is_empty())
                    .ok_or_else(|| de::Error::custom("header 认证需要非空 header 字段"))?;
                let prefix = s.prefix.filter(|p| !p.is_empty());
                match (env_opt, lit_opt) {
                    (Some(name), None) if !name.trim().is_empty() => Ok(Auth::Header {
                        header,
                        prefix,
                        value: SecretSource::EnvRef(name),
                    }),
                    (None, Some(v)) => Ok(Auth::Header {
                        header,
                        prefix,
                        value: SecretSource::Literal(v),
                    }),
                    (Some(_), Some(_)) => {
                        Err(de::Error::custom("token_env/value_env 与 token/value 只能二选一"))
                    }
                    _ => Err(de::Error::custom(
                        "header 认证需要 token_env（推荐）或 token",
                    )),
                }
            }
            _ => Err(de::Error::custom(format!(
                "暂只支持 bearer 或 header/api_key，收到 {kind}"
            ))),
        }
    }
}

impl Serialize for Auth {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Auth::Bearer { token } => {
                let mut m = s.serialize_map(Some(2))?;
                m.serialize_entry("type", "bearer")?;
                match token {
                    SecretSource::EnvRef(n) => m.serialize_entry("token_env", n)?,
                    SecretSource::Literal(v) => m.serialize_entry("token", v)?,
                }
                m.end()
            }
            Auth::Header {
                header,
                prefix,
                value,
            } => {
                let len = 3 + usize::from(prefix.is_some());
                let mut m = s.serialize_map(Some(len))?;
                m.serialize_entry("type", "header")?;
                m.serialize_entry("header", header)?;
                if let Some(p) = prefix {
                    m.serialize_entry("prefix", p)?;
                }
                match value {
                    SecretSource::EnvRef(n) => m.serialize_entry("token_env", n)?,
                    SecretSource::Literal(v) => m.serialize_entry("token", v)?,
                }
                m.end()
            }
        }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
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

    /// 按客户端命名空间（如 `cline`、`codex` 或通配 `"*"`）隔离的扩展字段透传槽位。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
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
            extensions: BTreeMap::new(),
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

    /// 提取经过必填项校验的强类型传输规格（消除各处分散的 `Option` 判空）。
    pub fn transport_spec(&self) -> crate::Result<TransportSpec> {
        match self.transport {
            Transport::Stdio => {
                let cmd = self
                    .command
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        crate::Error::Encode(format!("{}: stdio 条目缺少 command", self.name))
                    })?;
                Ok(TransportSpec::Stdio(StdioTransport {
                    command: cmd.to_string(),
                    args: self.args.clone(),
                    cwd: self.cwd.clone(),
                    timeout_seconds: self.timeout_seconds,
                }))
            }
            Transport::Http | Transport::Sse => {
                let url = self
                    .url
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        crate::Error::Encode(format!("{}: 远程条目缺少 url", self.name))
                    })?;
                let kind = match self.transport {
                    Transport::Sse => RemoteKind::Sse,
                    _ => RemoteKind::Http,
                };
                Ok(TransportSpec::Remote(RemoteTransport {
                    kind,
                    url: url.to_string(),
                    headers: self.headers.clone(),
                    auth: self.auth.clone(),
                    timeout_seconds: self.timeout_seconds,
                }))
            }
        }
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
