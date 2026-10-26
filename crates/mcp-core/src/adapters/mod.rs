//! 适配器层：IR ↔ 各客户端原生格式的双向转换。
//!
//! 绝大多数差异由声明式 `AdapterSpec` 描述，通用编码器统一处理；
//! 只有带方言的客户端才覆盖 `encode_entry` / `decode_entry`。

use crate::error::{Error, Result};
use crate::model::{Auth, McpEntry, Scope, SecretSource, Transport};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub mod clients;
pub mod paths;

pub use clients::{all, by_id, by_ids};

// ---------------------------------------------------------------- 声明式描述

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Jsonc,
    Toml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Cli,
    Ide,
    Desktop,
}

/// stdio 命令的表达方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryStyle {
    /// `{ command: "npx", args: [...] }`
    CommandArgs,
    /// `{ command: ["npx", "-y", "..."] }`（OpenCode）
    CommandArray,
}

/// 启用开关的语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnabledKey {
    /// 不支持开关，省略。
    None,
    /// `enabled: true/false`
    Enabled,
    /// `disabled: true`（语义反转，仅在禁用时写）
    Disabled,
}

/// bearer 密钥的落地能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStyle {
    /// 原生字段引用环境变量（Codex `bearer_token_env_var`）
    NativeEnvField,
    /// headers 内变量展开：前缀 + 变量名 + 后缀
    EnvExpansion(&'static str, &'static str),
    /// 只认字面值 → 触发 secret_policy
    LiteralOnly,
}

/// 声明式适配器描述。
#[derive(Debug)]
pub struct AdapterSpec {
    pub format: Format,
    /// 根键路径，如 ["mcpServers"] / ["mcp", "servers"] / ["mcp_servers"]
    pub root_key: &'static [&'static str],
    pub entry_style: EntryStyle,
    pub env_key: &'static str,
    pub remote_url_key: &'static str,
    pub type_field: Option<&'static str>,
    /// stdio 时 type 字段的取值（例如 OpenCode 用 "local"）
    pub stdio_type_value: Option<&'static str>,
    /// remote 时 type 字段的取值（例如 OpenCode 用 "remote"）
    pub remote_type_value: Option<&'static str>,
    pub enabled_key: EnabledKey,
    pub supports_cwd: bool,
    pub secret_style: SecretStyle,
    pub timeout_field: Option<&'static str>,
    pub scopes: &'static [Scope],
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Format::Json => "JSON",
            Format::Jsonc => "JSONC",
            Format::Toml => "TOML",
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
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn kind(&self) -> ClientKind;
    fn spec(&self) -> &AdapterSpec;

    /// 根键可随作用域变化（VS Code：用户级 `servers`，项目级 `mcpServers`）。
    fn root_key(&self, _scope: Scope) -> Vec<String> {
        self.spec().root_key.iter().map(|s| s.to_string()).collect()
    }

    fn supports_scope(&self, scope: Scope) -> bool {
        self.spec().scopes.contains(&scope)
    }

    fn encode_entry(&self, entry: &McpEntry) -> Result<Value> {
        generic_encode(self.spec(), entry)
    }

    fn decode_entry(&self, name: &str, value: &Value) -> Result<McpEntry> {
        generic_decode(self.spec(), name, value)
    }
}

// ---------------------------------------------------------------- 通用编码

const INLINE_PLACEHOLDER_PREFIX: &str = "<需内联:";

fn blocked_placeholder(env_name: &str) -> String {
    format!("{INLINE_PLACEHOLDER_PREFIX}{env_name}>")
}

fn apply_secret(style: SecretStyle, token: &SecretSource) -> AppliedSecret {
    match (style, token) {
        (SecretStyle::NativeEnvField, SecretSource::EnvRef(name)) => {
            AppliedSecret::NativeField(name.clone())
        }
        (SecretStyle::EnvExpansion(prefix, suffix), SecretSource::EnvRef(name)) => {
            AppliedSecret::Header(format!("Bearer {prefix}{name}{suffix}"))
        }
        (SecretStyle::LiteralOnly, SecretSource::EnvRef(name)) => {
            AppliedSecret::Header(format!("Bearer {}", blocked_placeholder(name)))
        }
        (_, SecretSource::Literal(v)) => AppliedSecret::Header(format!("Bearer {v}")),
    }
}

enum AppliedSecret {
    NativeField(String),
    Header(String),
}

/// 通用编码：把 IR 条目转成客户端的逻辑 JSON 结构。
pub fn generic_encode(spec: &AdapterSpec, entry: &McpEntry) -> Result<Value> {
    let mut obj = Map::new();

    if entry.is_remote() {
        let url = entry
            .url
            .as_deref()
            .ok_or_else(|| Error::Encode(format!("{}: 远程条目缺少 url", entry.name)))?;
        obj.insert(
            spec.remote_url_key.to_string(),
            Value::String(url.to_string()),
        );

        if let Some(field) = spec.type_field {
            let value = spec.remote_type_value.unwrap_or(match entry.transport {
                Transport::Sse => "sse",
                _ => "http",
            });
            obj.insert(field.to_string(), Value::String(value.to_string()));
        }
        if let (Some(field), Some(t)) = (spec.timeout_field, entry.timeout_seconds) {
            obj.insert(field.to_string(), Value::from(t));
        }
    } else {
        let cmd = entry
            .command
            .as_deref()
            .ok_or_else(|| Error::Encode(format!("{}: stdio 条目缺少 command", entry.name)))?;

        match spec.entry_style {
            EntryStyle::CommandArgs => {
                obj.insert("command".into(), Value::String(cmd.to_string()));
                if !entry.args.is_empty() {
                    obj.insert("args".into(), json!(entry.args));
                }
                if let (Some(field), Some(v)) = (spec.type_field, spec.stdio_type_value) {
                    obj.insert(field.to_string(), Value::String(v.to_string()));
                }
            }
            EntryStyle::CommandArray => {
                let mut arr: Vec<Value> = Vec::with_capacity(entry.args.len() + 1);
                arr.push(Value::String(cmd.to_string()));
                arr.extend(entry.args.iter().cloned().map(Value::String));
                obj.insert("command".into(), Value::Array(arr));
                if let Some(field) = spec.type_field {
                    let v = spec.stdio_type_value.unwrap_or("local");
                    obj.insert(field.to_string(), Value::String(v.to_string()));
                }
            }
        }

        if spec.supports_cwd {
            if let Some(cwd) = &entry.cwd {
                obj.insert("cwd".into(), Value::String(cwd.clone()));
            }
        }
    }

    if !entry.env.is_empty() {
        obj.insert(spec.env_key.to_string(), json!(entry.env));
    }

    // headers + 认证
    let mut headers = entry.headers.clone();
    if let Some(Auth::Bearer { token }) = &entry.auth {
        match apply_secret(spec.secret_style, token) {
            AppliedSecret::NativeField(env_name) => {
                obj.insert("bearer_token_env_var".into(), Value::String(env_name));
            }
            AppliedSecret::Header(h) => {
                headers.insert("Authorization".into(), h);
            }
        }
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

    Ok(Value::Object(obj))
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

    let url = str_of(obj, spec.remote_url_key)
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
    for key in [spec.env_key, "env", "environment"] {
        if let Some(Value::Object(m)) = obj.get(key) {
            entry.env = m
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect();
            break;
        }
    }

    if let Some(t) = obj
        .get(spec.timeout_field.unwrap_or("__none__"))
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
