//! 只读查看客户端已有 MCP：只返回目标子树，敏感值在离开后端前隐藏。
use crate::{App, AppError};
use mcp_core::adapters::Format;
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
        let adapter_set = self.adapter_set()?;
        let adapter = adapter_set
            .by_id(client_id)
            .ok_or_else(|| AppError::UnknownClient(client_id.to_string()))?;
        if let Some(reason) = self.check_scope_block(adapter, scope) {
            return Err(AppError::Other(reason));
        }
        let loc = self
            .target_location(adapter, scope)
            .ok_or_else(|| AppError::Other("缺少配置目录，无法定位当前作用域的配置".into()))?;
        let path = loc.path;
        // Grok 等客户端有 TOML / JSON 候选，必须按实际命中的文件解析。
        let format = adapter.format_for_path(&path);
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
            .entries(&loc.root_key)
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

fn redact(value: &Value) -> Value {
    mcp_core::redact::config_entry(value)
}
