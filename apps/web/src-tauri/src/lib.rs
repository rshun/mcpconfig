//! MCP Hub 桌面端：Tauri v2 薄适配层。
//!
//! 业务逻辑全在 `mcp-app`；本 crate 只注册 IPC 命令与窗口。

mod commands;
mod dto;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::detect,
            commands::read_client_config,
            commands::matrix,
            commands::load_catalog,
            commands::save_catalog,
            commands::plan,
            commands::apply,
            commands::backups,
            commands::restore,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
