use mcp_core::redact::{change, config_entry};
use serde_json::json;

#[test]
fn ordinary_addresses_packages_paths_and_arguments_remain_readable() {
    let value = json!({
        "command": "C:/Program Files/nodejs/npx.cmd",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "C:/My Documents", "--port", "8080", "--tokenizer", "cl100k_base", "--max-tokens=4096"],
        "cwd": "C:/My Projects/demo",
        "url": "http://localhost:8080/mcp?project=demo&limit=20",
        "env": {"PATH": "C:/Tools", "NODE_ENV": "production", "API_BASE_URL": "https://example.invalid/mcp", "MAX_TOKENS": "4096"},
        "headers": {"Accept": "application/json", "X-Project": "demo"},
        "custom": {"timeout": 30, "active": true, "notes": "ordinary text"}
    });
    assert_eq!(config_entry(&value), value);
}

#[test]
fn credentials_are_hidden_by_context_while_variable_references_remain_visible() {
    let value = json!({
        "command": ["npx", "-y", "@scope/mcp", "--api-key", "your_api_key_here", "--port=8080", "--password=your_password_here"],
        "env": {"SERVICE_TOKEN": "your_token_here", "PASSWORD": "your_password_here", "PIN": 1234, "API_KEY": "${ENV:MY_API_KEY}"},
        "headers": {"Authorization": "Bearer your_bearer_here", "X-Api-Key": "your_header_key_here", "Accept": "application/json"},
        "url": "https://example.invalid/mcp?api%5Fkey=your_query_key_here",
        "nested": {"clientSecret": "your_client_secret_here"}
    });
    let shown = config_entry(&value);
    for secret in [
        "your_api_key_here",
        "your_password_here",
        "your_token_here",
        "your_bearer_here",
        "your_header_key_here",
        "your_query_key_here",
        "your_client_secret_here",
        "1234",
    ] {
        assert!(
            !shown.to_string().contains(secret),
            "credential leaked: {secret}"
        );
    }
    assert_eq!(shown["command"][2], "@scope/mcp");
    assert_eq!(shown["command"][3], "--api-key");
    assert_eq!(shown["command"][5], "--port=8080");
    assert_eq!(shown["env"]["API_KEY"], "${ENV:MY_API_KEY}");
    assert_eq!(shown["headers"]["Accept"], "application/json");
}

#[test]
fn sensitive_changes_stay_visible_without_exposing_old_or_new_credentials() {
    let (old, new) = change(
        Some(&json!({"env":{"API_KEY":"your_old_key_here"},"args":["--port","8080"]})),
        &json!({"env":{"API_KEY":"your_new_key_here"},"args":["--port","8081"]}),
    );
    assert_eq!(old.unwrap()["env"]["API_KEY"], "[已隐藏：原值]");
    assert_eq!(new["env"]["API_KEY"], "[已隐藏：新值]");
    assert_eq!(new["args"][1], "8081");
}
