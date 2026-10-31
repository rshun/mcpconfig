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

#[test]
fn embedded_credentials_and_encoded_query_names_are_hidden_without_hiding_plain_urls() {
    for value in [
        "https://user:your_password_here@example.invalid/mcp",
        "https://user:your_password%2Fhere@example.invalid/mcp",
        "https://user:your_password%3Fhere@example.invalid/mcp",
        "https://example.invalid/mcp?token=%26",
        "https://example.invalid/mcp?access_token=your_token_here",
        "https://example.invalid/mcp?api%5Fkey=your_api_key_here",
        "https://example.invalid/mcp?X-Amz-Signature=your_signature_here",
        "--header=Authorization: Bearer your_token_here",
        "TOKEN=your_token_here",
        "npx @scope/mcp --password your_password_here",
        "--value=sk-your_placeholder_key_here_123",
    ] {
        assert_eq!(config_entry(&json!({"custom":value}))["custom"], "[已隐藏]");
    }
    let ordinary = json!({"url":"https://example.invalid/mcp?project=demo", "args":["--tokenizer=cl100k_base", "--max-tokens=4096"], "env":{"API_BASE_URL":"https://example.invalid","publicKey":"public-key-name"}});
    assert_eq!(config_entry(&ordinary), ordinary);
}

#[test]
fn existing_references_and_mixed_non_secret_text_are_kept_without_resolving_values() {
    let value = json!({
        "args":["--api-key", "${ENV:MY_KEY}", "--port", "8080"],
        "command":"npx @scope/mcp --api-key=${ENV:MY_KEY} --port=8080",
        "cwd":"${HOME}/My Projects",
        "url":"https://example.invalid/mcp?token=${ENV:MY_TOKEN}",
        "headers":{"Authorization":"Bearer ${ENV:MY_TOKEN}"},
        "auth":{"type":"bearer", "token_env":"MY_TOKEN"}
    });
    assert_eq!(config_entry(&value), value);
    assert_eq!(
        config_entry(&json!({"auth":{"type":"bearer", "value":"your_auth_value_here"}}))["auth"]
            ["value"],
        "[已隐藏]"
    );
    assert_eq!(
        config_entry(&json!({"env":{"API_KEY":"prefix-${ENV:MY_KEY}"}}))["env"]["API_KEY"],
        "[已隐藏]"
    );
}

#[test]
fn credential_file_paths_and_consumed_flag_values_do_not_hide_ordinary_values() {
    let value = json!({"env":{"PASSWORD_FILE":"C:/My Secrets/password.txt", "TOKEN_PATH":"/home/me/token"}, "args":["--token-file","C:/token.txt"]});
    assert_eq!(config_entry(&value), value);
    assert_eq!(
        config_entry(&json!({"args":["--password","--token","8080"]}))["args"],
        json!(["--password", "[已隐藏]", "8080"])
    );
}

#[test]
fn passwords_matching_auth_scheme_names_are_still_credentials() {
    for value in ["basic", "Bearer ", " BASIC ", "${ENV:MY_KEY}basic"] {
        assert_eq!(
            config_entry(&json!({"env":{"PASSWORD":value}}))["env"]["PASSWORD"],
            "[已隐藏]"
        );
    }
}
