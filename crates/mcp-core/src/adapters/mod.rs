//! 适配器层：IR ↔ 各客户端原生格式的双向转换。
//!
//! 绝大多数差异由声明式 `AdapterSpec` 描述，通用编码器统一处理；
//! 同时支持从外部 YAML/JSON 加载受安全约束的声明式 `CustomAdapter`。

use crate::error::{Error, Result};
use crate::model::{
    Auth, McpEntry, Platform, RemoteKind, Scope, SecretSource, Transport, TransportSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

pub mod clients;
pub mod paths;

pub use clients::{all, by_id, by_ids};

// ---------------------------------------------------------------- 声明式描述

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Json,
    Jsonc,
    Toml,
    Yaml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    Cli,
    Ide,
    Desktop,
}

/// stdio 命令的表达方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStyle {
    /// `{ command: "npx", args: [...] }`
    CommandArgs,
    /// `{ command: ["npx", "-y", "..."] }`（OpenCode）
    CommandArray,
}

/// 启用开关的语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnabledKey {
    /// 不支持开关，省略。
    None,
    /// `enabled: true/false`
    Enabled,
    /// `disabled: true`（语义反转，仅在禁用时写）
    Disabled,
}

/// 认证密钥的落地能力。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretStyle {
    /// 原生字段引用环境变量（Codex `bearer_token_env_var`）
    NativeEnvField,
    /// headers 内变量展开：前缀 + 变量名 + 后缀
    EnvExpansion(Cow<'static, str>, Cow<'static, str>),
    /// 只认字面值 → 需由作用域安全求值或阻断
    LiteralOnly,
}

/// 声明式适配器描述（支持静态内置与运行时加载两种生命周期）。
#[derive(Debug, Clone)]
pub struct AdapterSpec {
    pub format: Format,
    /// 根键路径，如 `["mcpServers"]` / `["mcp", "servers"]` / `["mcp_servers"]`
    pub root_key: Cow<'static, [&'static str]>,
    pub entry_style: EntryStyle,
    pub env_key: Cow<'static, str>,
    pub remote_url_key: Cow<'static, str>,
    pub type_field: Option<Cow<'static, str>>,
    /// stdio 时 type 字段的取值（例如 OpenCode 用 `"local"`）
    pub stdio_type_value: Option<Cow<'static, str>>,
    /// remote 时 type 字段的取值（例如 OpenCode 用 `"remote"`）
    pub remote_type_value: Option<Cow<'static, str>>,
    pub enabled_key: EnabledKey,
    pub supports_cwd: bool,
    pub secret_style: SecretStyle,
    pub timeout_field: Option<Cow<'static, str>>,
    pub scopes: Cow<'static, [Scope]>,
}

/// 结构化编码产物：携带客户端 JSON 表示及未解析的内联密钥变量名列表（取代魔术字符串扫描）。
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedEntry {
    pub value: Value,
    pub unresolved_inline_secrets: Vec<String>,
}

/// 允许无损导入与双向往返的已知安全客户端扩展字段白名单。
pub const KNOWN_CLIENT_EXTENSION_KEYS: &[&str] = &[
    "alwaysAllow",
    "autoApprove",
    "disabledTools",
    "tool_timeout_sec",
];

/// 核心协议保留键（禁止被 `extensions` 覆盖）。
pub const RESERVED_EXTENSION_KEYS: &[&str] = &[
    "command",
    "args",
    "env",
    "environment",
    "cwd",
    "url",
    "serverUrl",
    "headers",
    "type",
    "enabled",
    "disabled",
    "bearer_token_env_var",
    "startup_timeout_sec",
    "timeoutSeconds",
];

pub fn is_reserved_extension_key(key: &str) -> bool {
    RESERVED_EXTENSION_KEYS.contains(&key)
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Format::Json => "JSON",
            Format::Jsonc => "JSONC",
            Format::Toml => "TOML",
            Format::Yaml => "YAML",
        };
        write!(f, "{s}")
    }
}

impl std::fmt::Display for ClientKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ClientKind::Cli => "CLI",
            ClientKind::Ide => "IDE",
            ClientKind::Desktop => "桌面",
        };
        write!(f, "{s}")
    }
}

// ---------------------------------------------------------------- 适配器抽象

pub trait Adapter: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn kind(&self) -> ClientKind;
    fn spec(&self) -> &AdapterSpec;

    /// 返回该适配器在指定作用域与平台下的候选路径模板（按优先级排序）。
    fn path_candidates(&self, scope: Scope, platform: Platform) -> Vec<String> {
        paths::candidates(self.id(), scope, platform)
    }

    /// 根键可随作用域变化（VS Code：用户级 `servers`，项目级 `mcpServers`）。
    fn root_key(&self, _scope: Scope) -> Vec<String> {
        self.spec().root_key.iter().map(|s| s.to_string()).collect()
    }

    /// 多候选文件可能使用不同格式，不能仅由作用域决定。
    fn root_key_for_path(&self, scope: Scope, _path: &std::path::Path) -> Vec<String> {
        self.root_key(scope)
    }

    fn format_for_path(&self, path: &std::path::Path) -> Format {
        match path.extension().and_then(|s| s.to_str()) {
            Some("toml") => Format::Toml,
            Some("jsonc") => Format::Jsonc,
            Some("yaml" | "yml") => Format::Yaml,
            Some("json") if matches!(self.spec().format, Format::Toml | Format::Yaml) => {
                Format::Json
            }
            _ => self.spec().format,
        }
    }

    fn supports_scope(&self, scope: Scope) -> bool {
        self.spec().scopes.contains(&scope)
    }

    fn encode_entry_checked(&self, entry: &McpEntry) -> Result<EncodedEntry> {
        let mut encoded = generic_encode_checked(self.spec(), entry)?;
        if let Some(obj) = encoded.value.as_object_mut() {
            apply_extensions(obj, self.id(), self.spec(), &entry.extensions);
        }
        Ok(encoded)
    }

    fn encode_entry(&self, entry: &McpEntry) -> Result<Value> {
        self.encode_entry_checked(entry).map(|e| e.value)
    }

    fn decode_entry(&self, name: &str, value: &Value) -> Result<McpEntry> {
        let mut entry = generic_decode(self.spec(), name, value)?;
        if let Some(obj) = value.as_object() {
            extract_known_extensions(obj, self.id(), self.spec(), &mut entry);
        }
        Ok(entry)
    }
}

fn apply_extensions(
    obj: &mut Map<String, Value>,
    client_id: &str,
    spec: &AdapterSpec,
    extensions: &BTreeMap<String, BTreeMap<String, Value>>,
) {
    for bucket in ["*", client_id] {
        if let Some(ext) = extensions.get(bucket) {
            for (k, v) in ext {
                if is_reserved_extension_key(k)
                    || k == spec.env_key.as_ref()
                    || k == spec.remote_url_key.as_ref()
                    || spec.type_field.as_deref() == Some(k.as_str())
                    || spec.timeout_field.as_deref() == Some(k.as_str())
                {
                    continue;
                }
                obj.insert(k.clone(), v.clone());
            }
        }
    }
}

fn extract_known_extensions(
    obj: &Map<String, Value>,
    client_id: &str,
    spec: &AdapterSpec,
    entry: &mut McpEntry,
) {
    let mut bucket = BTreeMap::new();
    for &key in KNOWN_CLIENT_EXTENSION_KEYS {
        if spec.timeout_field.as_deref() == Some(key) {
            continue;
        }
        let Some(val) = obj.get(key) else { continue };
        let valid = match key {
            "alwaysAllow" | "autoApprove" | "disabledTools" => val
                .as_array()
                .is_some_and(|arr| arr.iter().all(|item| item.is_string())),
            "tool_timeout_sec" => val.as_u64().is_some(),
            _ => false,
        };
        if valid {
            bucket.insert(key.to_string(), val.clone());
        }
    }
    if !bucket.is_empty() {
        entry.extensions.insert(client_id.to_string(), bucket);
    }
}

// ---------------------------------------------------------------- 通用编码

pub const INLINE_PLACEHOLDER_PREFIX: &str = "<需内联:";

fn blocked_placeholder(env_name: &str) -> String {
    format!("{INLINE_PLACEHOLDER_PREFIX}{env_name}>")
}

fn apply_bearer_secret(style: &SecretStyle, token: &SecretSource) -> AppliedSecret {
    match (style, token) {
        (SecretStyle::NativeEnvField, SecretSource::EnvRef(name)) => {
            AppliedSecret::NativeField(name.clone())
        }
        (SecretStyle::EnvExpansion(prefix, suffix), SecretSource::EnvRef(name)) => {
            AppliedSecret::Header(format!("Bearer {prefix}{name}{suffix}"), None)
        }
        (SecretStyle::LiteralOnly, SecretSource::EnvRef(name)) => AppliedSecret::Header(
            format!("Bearer {}", blocked_placeholder(name)),
            Some(name.clone()),
        ),
        (_, SecretSource::Literal(v)) => AppliedSecret::Header(format!("Bearer {v}"), None),
    }
}

enum AppliedSecret {
    NativeField(String),
    Header(String, Option<String>),
}

/// 通用结构化编码：把 IR 条目转成客户端逻辑 JSON 结构并显式收集未决内联密钥。
pub fn generic_encode_checked(spec: &AdapterSpec, entry: &McpEntry) -> Result<EncodedEntry> {
    let mut obj = Map::new();
    let mut unresolved_inline_secrets = Vec::new();

    match entry.transport_spec()? {
        TransportSpec::Remote(remote) => {
            obj.insert(
                spec.remote_url_key.to_string(),
                Value::String(remote.url),
            );

            if let Some(field) = &spec.type_field {
                let value = spec
                    .remote_type_value
                    .as_deref()
                    .unwrap_or(match remote.kind {
                        RemoteKind::Sse => "sse",
                        RemoteKind::Http => "http",
                    });
                obj.insert(field.to_string(), Value::String(value.to_string()));
            }
            if let (Some(field), Some(t)) = (&spec.timeout_field, remote.timeout_seconds) {
                obj.insert(field.to_string(), Value::from(t));
            }
        }
        TransportSpec::Stdio(stdio) => {
            match spec.entry_style {
                EntryStyle::CommandArgs => {
                    obj.insert("command".into(), Value::String(stdio.command));
                    if !stdio.args.is_empty() {
                        obj.insert("args".into(), json!(stdio.args));
                    }
                    if let (Some(field), Some(v)) = (&spec.type_field, &spec.stdio_type_value) {
                        obj.insert(field.to_string(), Value::String(v.to_string()));
                    }
                }
                EntryStyle::CommandArray => {
                    let mut arr: Vec<Value> = Vec::with_capacity(stdio.args.len() + 1);
                    arr.push(Value::String(stdio.command));
                    arr.extend(stdio.args.into_iter().map(Value::String));
                    obj.insert("command".into(), Value::Array(arr));
                    if let Some(field) = &spec.type_field {
                        let v = spec.stdio_type_value.as_deref().unwrap_or("local");
                        obj.insert(field.to_string(), Value::String(v.to_string()));
                    }
                }
            }

            if spec.supports_cwd {
                if let Some(cwd) = stdio.cwd {
                    obj.insert("cwd".into(), Value::String(cwd));
                }
            }
            if let (Some(field), Some(t)) = (&spec.timeout_field, stdio.timeout_seconds) {
                obj.insert(field.to_string(), Value::from(t));
            }
        }
    }

    if !entry.env.is_empty() {
        obj.insert(spec.env_key.to_string(), json!(entry.env));
    }

    // headers + 认证
    let mut headers = entry.headers.clone();
    match &entry.auth {
        Some(Auth::Bearer { token }) => match apply_bearer_secret(&spec.secret_style, token) {
            AppliedSecret::NativeField(env_name) => {
                obj.insert("bearer_token_env_var".into(), Value::String(env_name));
            }
            AppliedSecret::Header(h, unresolved) => {
                if let Some(var) = unresolved {
                    unresolved_inline_secrets.push(var);
                }
                headers.insert("Authorization".into(), h);
            }
        },
        Some(Auth::Header {
            header,
            prefix,
            value,
        }) => {
            let pfx = prefix
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(|p| format!("{} ", p.trim_end()))
                .unwrap_or_default();
            match (&spec.secret_style, value) {
                (SecretStyle::EnvExpansion(ep, es), SecretSource::EnvRef(name)) => {
                    headers.insert(header.clone(), format!("{pfx}{ep}{name}{es}"));
                }
                (_, SecretSource::EnvRef(name)) => {
                    unresolved_inline_secrets.push(name.clone());
                    headers.insert(header.clone(), format!("{pfx}{}", blocked_placeholder(name)));
                }
                (_, SecretSource::Literal(v)) => {
                    headers.insert(header.clone(), format!("{pfx}{v}"));
                }
            }
        }
        None => {}
    }
    if !headers.is_empty() {
        obj.insert("headers".into(), json!(headers));
    }

    match spec.enabled_key {
        EnabledKey::None => {}
        EnabledKey::Enabled => {
            obj.insert("enabled".into(), Value::Bool(entry.enabled));
        }
        EnabledKey::Disabled => {
            if !entry.enabled {
                obj.insert("disabled".into(), Value::Bool(true));
            }
        }
    }

    Ok(EncodedEntry {
        value: Value::Object(obj),
        unresolved_inline_secrets,
    })
}

/// 通用编码：把 IR 条目转成客户端的逻辑 JSON 结构。
pub fn generic_encode(spec: &AdapterSpec, entry: &McpEntry) -> Result<Value> {
    generic_encode_checked(spec, entry).map(|e| e.value)
}

// ---------------------------------------------------------------- 通用解码

fn parse_bearer_header(raw: &str) -> Option<SecretSource> {
    let rest = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?;
    let rest = rest.trim();

    if let Some(inner) = rest.strip_prefix("${env:") {
        if let Some(name) = inner.strip_suffix('}') {
            return Some(SecretSource::EnvRef(name.trim().to_string()));
        }
    }
    if let Some(inner) = rest.strip_prefix("{env:") {
        if let Some(name) = inner.strip_suffix('}') {
            return Some(SecretSource::EnvRef(name.trim().to_string()));
        }
    }
    if let Some(inner) = rest.strip_prefix("${") {
        if let Some(name) = inner.strip_suffix('}') {
            // ${VAR} 与 ${ENV:VAR} 都归一为变量名
            let name = name.trim().strip_prefix("ENV:").unwrap_or(name.trim());
            return Some(SecretSource::EnvRef(name.to_string()));
        }
    }
    if let Some(inner) = rest.strip_prefix(INLINE_PLACEHOLDER_PREFIX) {
        if let Some(name) = inner.strip_suffix('>') {
            return Some(SecretSource::EnvRef(name.trim().to_string()));
        }
    }
    if rest.is_empty() {
        return None;
    }
    Some(SecretSource::Literal(rest.to_string()))
}

fn str_of(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// 通用解码：客户端原生结构 → IR。
pub fn generic_decode(spec: &AdapterSpec, name: &str, value: &Value) -> Result<McpEntry> {
    let obj = value
        .as_object()
        .ok_or_else(|| Error::Decode(format!("{name}: 条目不是对象")))?;

    let url = str_of(obj, spec.remote_url_key.as_ref())
        .or_else(|| str_of(obj, "url"))
        .or_else(|| str_of(obj, "serverUrl"));

    let mut entry = McpEntry::new_stdio(name, "", Vec::new());

    if let Some(url) = url {
        entry.transport = match str_of(obj, "type").as_deref() {
            Some("sse") => Transport::Sse,
            _ => Transport::Http,
        };
        entry.url = Some(url);
        entry.command = None;
    } else {
        entry.transport = Transport::Stdio;
        if let Some(cmd) = obj.get("command") {
            match cmd {
                Value::String(s) => {
                    entry.command = Some(s.clone());
                    entry.args = obj
                        .get("args")
                        .and_then(|a| a.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                }
                Value::Array(arr) => {
                    let mut iter = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string()));
                    entry.command = iter.next();
                    entry.args = iter.collect();
                }
                _ => {}
            }
        }
        entry.cwd = str_of(obj, "cwd");
    }

    // env：按 spec 的键，兼容两种拼写
    for key in [spec.env_key.as_ref(), "env", "environment"] {
        if let Some(Value::Object(m)) = obj.get(key) {
            entry.env = m
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect();
            break;
        }
    }

    if let Some(t) = obj
        .get(spec.timeout_field.as_deref().unwrap_or("__none__"))
        .and_then(|v| v.as_u64())
    {
        entry.timeout_seconds = Some(t);
    }

    // 开关
    match spec.enabled_key {
        EnabledKey::None => entry.enabled = true,
        EnabledKey::Enabled => {
            entry.enabled = obj.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true)
        }
        EnabledKey::Disabled => {
            entry.enabled = !obj
                .get("disabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        }
    }

    // 认证：先收集普通 headers（Authorization 归一为 auth），再按原生字段覆盖
    if let Some(Value::Object(m)) = obj.get("headers") {
        let mut headers: BTreeMap<String, String> = BTreeMap::new();
        for (k, v) in m {
            let Some(val) = v.as_str() else { continue };
            if k.eq_ignore_ascii_case("authorization") {
                if let Some(token) = parse_bearer_header(val) {
                    entry.auth = Some(Auth::Bearer { token });
                }
                continue;
            }
            headers.insert(k.clone(), val.to_string());
        }
        entry.headers = headers;
    }
    if let Some(env_name) = str_of(obj, "bearer_token_env_var") {
        entry.auth = Some(Auth::Bearer {
            token: SecretSource::EnvRef(env_name),
        });
    }

    Ok(entry)
}

// ---------------------------------------------------------------- 声明式自定义适配器（CustomAdapter）

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomAdaptersFile {
    #[serde(default = "default_custom_version")]
    version: u32,
    #[serde(default)]
    adapters: Vec<CustomAdapterDef>,
}

fn default_custom_version() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CustomSecretStyle {
    LiteralOnly,
    NativeEnvField,
    EnvExpansion { prefix: String, suffix: String },
}

impl Default for CustomSecretStyle {
    fn default() -> Self {
        Self::LiteralOnly
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformPaths {
    #[serde(default)]
    all: Vec<String>,
    #[serde(default)]
    windows: Vec<String>,
    #[serde(default)]
    linux: Vec<String>,
    #[serde(default)]
    macos: Vec<String>,
}

impl PlatformPaths {
    fn for_platform(&self, platform: Platform) -> Vec<String> {
        let specific = match platform {
            Platform::Windows => &self.windows,
            Platform::Linux => &self.linux,
            Platform::Macos => &self.macos,
        };
        if !specific.is_empty() {
            specific.clone()
        } else {
            self.all.clone()
        }
    }

    fn all_templates(&self) -> impl Iterator<Item = &str> {
        self.all
            .iter()
            .chain(&self.windows)
            .chain(&self.linux)
            .chain(&self.macos)
            .map(String::as_str)
    }

    fn is_configured(&self) -> bool {
        self.all_templates().next().is_some()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomScopePaths {
    #[serde(default)]
    user: PlatformPaths,
    #[serde(default)]
    project: PlatformPaths,
    #[serde(default)]
    local: PlatformPaths,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomAdapterDef {
    id: String,
    #[serde(alias = "display_name")]
    name: String,
    #[serde(default = "default_client_kind")]
    kind: ClientKind,
    #[serde(default = "default_format")]
    format: Format,
    root_key: Vec<String>,
    #[serde(default = "default_entry_style")]
    entry_style: EntryStyle,
    #[serde(default = "default_env_key")]
    env_key: String,
    #[serde(default = "default_remote_url_key")]
    remote_url_key: String,
    #[serde(default)]
    type_field: Option<String>,
    #[serde(default)]
    stdio_type_value: Option<String>,
    #[serde(default)]
    remote_type_value: Option<String>,
    #[serde(default = "default_enabled_key")]
    enabled_key: EnabledKey,
    #[serde(default)]
    supports_cwd: bool,
    #[serde(default)]
    secret_style: CustomSecretStyle,
    #[serde(default)]
    timeout_field: Option<String>,
    paths: CustomScopePaths,
}

fn default_client_kind() -> ClientKind {
    ClientKind::Cli
}
fn default_format() -> Format {
    Format::Json
}
fn default_entry_style() -> EntryStyle {
    EntryStyle::CommandArgs
}
fn default_env_key() -> String {
    "env".to_string()
}
fn default_remote_url_key() -> String {
    "url".to_string()
}
fn default_enabled_key() -> EnabledKey {
    EnabledKey::None
}

/// 运行时加载的声明式自定义适配器。
#[derive(Debug, Clone)]
pub struct CustomAdapter {
    id: String,
    name: String,
    kind: ClientKind,
    root_key: Vec<String>,
    spec: AdapterSpec,
    paths: CustomScopePaths,
}

impl Adapter for CustomAdapter {
    fn id(&self) -> &str {
        &self.id
    }
    fn display_name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> ClientKind {
        self.kind
    }
    fn spec(&self) -> &AdapterSpec {
        &self.spec
    }
    fn root_key(&self, _scope: Scope) -> Vec<String> {
        self.root_key.clone()
    }
    fn path_candidates(&self, scope: Scope, platform: Platform) -> Vec<String> {
        match scope {
            Scope::User => self.paths.user.for_platform(platform),
            Scope::Project => self.paths.project.for_platform(platform),
            Scope::Local => self.paths.local.for_platform(platform),
        }
    }
}

fn valid_custom_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn validate_path_template(id: &str, scope: Scope, tpl: &str) -> Result<()> {
    let trimmed = tpl.trim();
    if trimmed.is_empty() || trimmed != tpl {
        return Err(Error::Invalid(format!(
            "自定义适配器 {id} 存在空或含首尾空白的路径模板"
        )));
    }
    let file_part = trimmed.split_once('#').map(|(p, _)| p).unwrap_or(trimmed);
    if file_part.split(['/', '\\']).any(|seg| seg == "..") {
        return Err(Error::Invalid(format!(
            "自定义适配器 {id} 路径模板禁止包含 `..`：{trimmed}"
        )));
    }
    let valid_prefix = match scope {
        Scope::User | Scope::Local => {
            file_part.starts_with("~/")
                || file_part.starts_with("%USERPROFILE%/")
                || file_part.starts_with("%APPDATA%/")
        }
        Scope::Project => file_part.starts_with("<proj>/"),
    };
    if !valid_prefix {
        return Err(Error::Invalid(format!(
            "自定义适配器 {id} 的 {} 路径模板必须使用受控前缀（`~/`、`%USERPROFILE%/`、`%APPDATA%/` 或 `<proj>/`）：{trimmed}",
            scope.as_str()
        )));
    }
    Ok(())
}

/// 解析并严格校验自定义适配器声明文件（YAML 或 JSON）。
pub fn parse_custom_adapters(src: &str) -> Result<Vec<CustomAdapter>> {
    if src.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: CustomAdaptersFile = serde_yaml::from_str(src).map_err(|e| {
        let loc = e
            .location()
            .map(|l| format!("（行 {}，列 {}）", l.line(), l.column()))
            .unwrap_or_default();
        Error::Parse(format!("自定义适配器配置解析失败{loc}"))
    })?;
    if parsed.version != 1 {
        return Err(Error::Invalid(format!(
            "不支持的自定义适配器配置版本：{}",
            parsed.version
        )));
    }

    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(parsed.adapters.len());

    for def in parsed.adapters {
        if !valid_custom_id(&def.id) || def.id == "catalog" {
            return Err(Error::Invalid(format!(
                "自定义适配器 id 非法或为保留字：{}",
                def.id
            )));
        }
        if clients::by_id(&def.id).is_some() {
            return Err(Error::Invalid(format!(
                "自定义适配器 id 与内置客户端冲突：{}",
                def.id
            )));
        }
        if !seen.insert(def.id.clone()) {
            return Err(Error::Invalid(format!("自定义适配器 id 重复：{}", def.id)));
        }
        if def.name.trim().is_empty() {
            return Err(Error::Invalid(format!("自定义适配器 {} 缺少名称", def.id)));
        }
        if def.root_key.is_empty() || def.root_key.iter().any(|s| s.trim().is_empty()) {
            return Err(Error::Invalid(format!(
                "自定义适配器 {} 的 root_key 不能为空",
                def.id
            )));
        }

        let mut scopes = Vec::new();
        if def.paths.user.is_configured() {
            for t in def.paths.user.all_templates() {
                validate_path_template(&def.id, Scope::User, t)?;
            }
            scopes.push(Scope::User);
        }
        if def.paths.project.is_configured() {
            for t in def.paths.project.all_templates() {
                validate_path_template(&def.id, Scope::Project, t)?;
            }
            scopes.push(Scope::Project);
        }
        if def.paths.local.is_configured() {
            for t in def.paths.local.all_templates() {
                validate_path_template(&def.id, Scope::Local, t)?;
            }
            scopes.push(Scope::Local);
        }
        if scopes.is_empty() {
            return Err(Error::Invalid(format!(
                "自定义适配器 {} 至少需声明一个作用域的路径模板",
                def.id
            )));
        }

        let secret_style = match def.secret_style {
            CustomSecretStyle::LiteralOnly => SecretStyle::LiteralOnly,
            CustomSecretStyle::NativeEnvField => SecretStyle::NativeEnvField,
            CustomSecretStyle::EnvExpansion { prefix, suffix } => {
                if prefix.is_empty() {
                    return Err(Error::Invalid(format!(
                        "自定义适配器 {} 的 env_expansion.prefix 不能为空",
                        def.id
                    )));
                }
                SecretStyle::EnvExpansion(Cow::Owned(prefix), Cow::Owned(suffix))
            }
        };

        let spec = AdapterSpec {
            format: def.format,
            root_key: Cow::Borrowed(&[]),
            entry_style: def.entry_style,
            env_key: Cow::Owned(def.env_key),
            remote_url_key: Cow::Owned(def.remote_url_key),
            type_field: def.type_field.map(Cow::Owned),
            stdio_type_value: def.stdio_type_value.map(Cow::Owned),
            remote_type_value: def.remote_type_value.map(Cow::Owned),
            enabled_key: def.enabled_key,
            supports_cwd: def.supports_cwd,
            secret_style,
            timeout_field: def.timeout_field.map(Cow::Owned),
            scopes: Cow::Owned(scopes),
        };

        out.push(CustomAdapter {
            id: def.id,
            name: def.name,
            kind: def.kind,
            root_key: def.root_key,
            spec,
            paths: def.paths,
        });
    }

    Ok(out)
}
