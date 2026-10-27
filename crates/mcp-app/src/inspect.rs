//! 只读查看客户端已有 MCP：只返回目标子树，敏感值在离开后端前隐藏。
use crate::{scope_block_reason, App, AppError};
use mcp_core::adapters::{self, Format};
use mcp_core::document::ConfigDoc;
use mcp_core::model::Scope;
use serde_json::Value;

#[derive(Debug)]
pub struct ExistingEntry {
    pub name: String,
    pub config: Value,
}

#[derive(Debug)]
pub struct ExistingConfig {
    pub client_id: String,
    pub client_name: String,
    pub scope: Scope,
    pub path: String,
    pub format: Format,
    pub existed: bool,
    pub entries: Vec<ExistingEntry>,
}

impl App {
    pub fn read_client_config(
        &self,
        client_id: &str,
        scope: Scope,
    ) -> Result<ExistingConfig, AppError> {
        let adapter = adapters::by_id(client_id)
            .ok_or_else(|| AppError::UnknownClient(client_id.to_string()))?;
        if let Some(reason) = scope_block_reason(client_id, scope) {
            return Err(AppError::Other(reason));
        }
        let path = self
            .target_path(adapter, scope)
            .ok_or_else(|| AppError::Other("缺少配置目录，无法定位当前作用域的配置".into()))?;
        // Grok 等客户端有 TOML / JSON 候选，必须按实际命中的文件解析。
        let format = match path.extension().and_then(|ext| ext.to_str()) {
            Some("toml") => Format::Toml,
            Some("json") => Format::Json,
            Some("jsonc") => Format::Jsonc,
            _ => adapter.spec().format,
        };
        let mut result = ExistingConfig {
            client_id: client_id.to_string(),
            client_name: adapter.display_name().to_string(),
            scope,
            path: path.display().to_string(),
            format,
            existed: false,
            entries: Vec::new(),
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(result),
            Err(error) => {
                return Err(AppError::Other(format!(
                    "无法读取配置文件（{:?}），请检查权限和文件编码",
                    error.kind()
                )))
            }
        };
        result.existed = true;
        // 解析错误可能含 TOML 源码和真实凭据，不能把底层错误传给 IPC。
        let doc = ConfigDoc::parse(format, text.strip_prefix('\u{feff}').unwrap_or(&text))
            .map_err(|_| {
                AppError::Other(format!(
                    "{} 配置解析失败，请检查文件格式；未显示原文",
                    adapter.display_name()
                ))
            })?;
        let entries = doc
            .entries(&adapter.root_key(scope))
            .map_err(|_| AppError::Other("MCP 配置容器结构无效，应为对象或表".into()))?;
        for (name, entry) in entries {
            if !entry.is_object() {
                return Err(AppError::Other("MCP 条目结构无效，应为对象或表".into()));
            }
            result.entries.push(ExistingEntry {
                name,
                config: redact(&entry),
            });
        }
        result.entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }
}

const HIDDEN: &str = "[已隐藏]";

// 采用白名单展示：未知字符串也隐藏，避免凭据通过自定义字段、参数或 URL 泄露。
fn redact(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let safe = match (key.as_str(), value) {
                        ("command", Value::String(command)) if safe_command(command) => {
                            value.clone()
                        }
                        ("command", Value::Array(parts)) => Value::Array(
                            parts
                                .iter()
                                .enumerate()
                                .map(|(i, part)| {
                                    if i == 0 && part.as_str().is_some_and(safe_command) {
                                        part.clone()
                                    } else {
                                        hide_values(part)
                                    }
                                })
                                .collect(),
                        ),
                        ("type" | "transport", Value::String(kind))
                            if ["stdio", "http", "sse", "local", "remote"]
                                .contains(&kind.as_str()) =>
                        {
                            value.clone()
                        }
                        ("enabled" | "disabled", Value::Bool(_)) => value.clone(),
                        _ => hide_values(value),
                    };
                    (key.clone(), safe)
                })
                .collect(),
        ),
        _ => hide_values(value),
    }
}

fn safe_command(command: &str) -> bool {
    !command.is_empty()
        && command
            .chars()
            .all(|c| c.is_alphanumeric() || "_./\\:-".contains(c))
}

fn hide_values(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(k, v)| (k.clone(), hide_values(v)))
                .collect(),
        ),
        Value::Array(array) => Value::Array(array.iter().map(hide_values).collect()),
        // 环境变量的数值、bool 也可能是密码，统一隐藏，保留字段和数组结构。
        _ => Value::String(HIDDEN.into()),
    }
}
