//! # mcp-core
//!
//! MCP Hub 的核心层：**纯逻辑，无 IO、无全局状态**。
//!
//! - `model`：规范化中间模型（IR）
//! - `catalog`：客户清单解析与校验
//! - `interpolate`：`${ENV:VAR}` / `${HOME}` / `${PROJECT_ROOT}` 插值
//! - `resolve`：平台覆盖合并 + 插值 + 生效作用域
//! - `adapters`：14 个客户端的编码 / 解码
//!
//! 本 crate **不得**依赖 tauri / clap 等宿主库（CI 用 cargo tree 断言）。

pub mod adapters;
pub mod catalog;
pub mod diff;
pub mod document;
pub mod error;
pub mod interpolate;
pub mod model;
pub mod plan;
pub mod redact;
pub mod resolve;

pub use error::{Error, Result};
pub use model::{Auth, Catalog, EntryOverride, McpEntry, Platform, Scope, SecretSource, Transport};

use serde_json::Value;

/// 全部客户端 id（顺序即 UI 列顺序）。
pub fn client_ids() -> Vec<&'static str> {
    adapters::all().iter().map(|a| a.id()).collect()
}

/// 按客户端 id 编码单条配置。未知 id 返回错误。
pub fn encode_entry(client_id: &str, entry: &McpEntry) -> Result<Value> {
    let adapter =
        adapters::by_id(client_id).ok_or_else(|| Error::UnknownClient(client_id.to_string()))?;
    adapter.encode_entry(entry)
}

/// 按客户端 id 解码单条配置。
pub fn decode_entry(client_id: &str, name: &str, value: &Value) -> Result<McpEntry> {
    let adapter =
        adapters::by_id(client_id).ok_or_else(|| Error::UnknownClient(client_id.to_string()))?;
    adapter.decode_entry(name, value)
}
