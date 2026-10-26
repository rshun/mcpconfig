//! 适配器往返测试：IR → 客户端格式 → IR 必须等价。
//!
//! 覆盖 14 个客户端 × stdio / remote × bearer 三档。

use mcp_core::adapters;
use mcp_core::model::{Auth, McpEntry, Scope, SecretSource, Transport};
use std::collections::BTreeMap;

fn stdio_entry() -> McpEntry {
    let mut e = McpEntry::new_stdio("stockdb", "python", vec!["-u".into(), "s.py".into()]);
    e.env.insert("DB_URL".into(), "postgres://x".into());
    e
}

fn http_entry() -> McpEntry {
    let mut e = McpEntry::new_http("github", "https://api.githubcopilot.com/mcp/");
    e.auth = Some(Auth::env_ref("GITHUB_PAT"));
    e
}

// ---------------------------------------------------------------- 覆盖完整性

#[test]
fn has_exactly_14_clients() {
    assert_eq!(adapters::all().len(), 14, "客户端数量应为 14");
}

#[test]
fn expected_client_ids_present() {
    let ids: Vec<&str> = adapters::all().iter().map(|a| a.id()).collect();
    for want in [
        "claude-code",
        "codex",
        "workbuddy",
        "kimi-code",
        "opencode",
        "vscode",
        "antigravity",
        "grok",
        "cursor",
        "cline",
        "gemini-cli",
        "claude-desktop",
        "windsurf",
        "trae",
    ] {
        assert!(ids.contains(&want), "缺少客户端 {want}");
    }
}

#[test]
fn unknown_client_errors() {
    assert!(mcp_core::encode_entry("nope", &stdio_entry()).is_err());
}

// ---------------------------------------------------------------- stdio 往返

#[test]
fn stdio_round_trip_all_clients() {
    for a in adapters::all() {
        let e = stdio_entry();
        let v = a
            .encode_entry(&e)
            .unwrap_or_else(|err| panic!("{}: {err}", a.id()));
        let back = a
            .decode_entry(&e.name, &v)
            .unwrap_or_else(|err| panic!("{}: {err}", a.id()));

        assert_eq!(back.transport, Transport::Stdio, "{} transport", a.id());
        assert_eq!(back.command, e.command, "{} command", a.id());
        assert_eq!(back.args, e.args, "{} args", a.id());
        assert_eq!(back.env, e.env, "{} env", a.id());
        assert_eq!(back.name, e.name, "{} name", a.id());
    }
}

#[test]
fn remote_round_trip_all_clients() {
    for a in adapters::all() {
        let e = http_entry();
        let v = a.encode_entry(&e).unwrap();
        let back = a.decode_entry(&e.name, &v).unwrap();
        assert_eq!(back.transport, Transport::Http, "{} transport", a.id());
        assert_eq!(back.url, e.url, "{} url", a.id());
        assert_eq!(
            back.auth.as_ref().and_then(|x| x.env_name()),
            Some("GITHUB_PAT"),
            "{} bearer 归一",
            a.id()
        );
    }
}

#[test]
fn cwd_only_kept_where_supported() {
    for a in adapters::all() {
        let mut e = stdio_entry();
        e.cwd = Some("/work/proj".into());
        let v = a.encode_entry(&e).unwrap();
        let back = a.decode_entry(&e.name, &v).unwrap();
        if a.spec().supports_cwd {
            assert_eq!(
                back.cwd.as_deref(),
                Some("/work/proj"),
                "{} 应保留 cwd",
                a.id()
            );
        } else {
            assert_eq!(back.cwd, None, "{} 不应写入 cwd", a.id());
        }
    }
}

// ---------------------------------------------------------------- bearer 三档

#[test]
fn codex_uses_native_env_field() {
    let v = mcp_core::encode_entry("codex", &http_entry()).unwrap();
    assert_eq!(v["bearer_token_env_var"], "GITHUB_PAT");
    assert!(v.get("headers").is_none(), "Codex 不应写入明文 header");
    assert_eq!(v["url"], "https://api.githubcopilot.com/mcp/");
}

#[test]
fn claude_code_uses_dollar_expansion() {
    let v = mcp_core::encode_entry("claude-code", &http_entry()).unwrap();
    assert_eq!(v["headers"]["Authorization"], "Bearer ${GITHUB_PAT}");
}

#[test]
fn grok_uses_dollar_expansion() {
    let v = mcp_core::encode_entry("grok", &http_entry()).unwrap();
    assert_eq!(v["headers"]["Authorization"], "Bearer ${GITHUB_PAT}");
}

#[test]
fn opencode_uses_brace_env_syntax() {
    let v = mcp_core::encode_entry("opencode", &http_entry()).unwrap();
    assert_eq!(v["headers"]["Authorization"], "Bearer {env:GITHUB_PAT}");
}

#[test]
fn windsurf_uses_dollar_env_syntax() {
    let v = mcp_core::encode_entry("windsurf", &http_entry()).unwrap();
    assert_eq!(v["headers"]["Authorization"], "Bearer ${env:GITHUB_PAT}");
}

#[test]
fn literal_only_clients_emit_inline_placeholder() {
    for id in ["workbuddy", "antigravity", "vscode", "cursor", "trae"] {
        let v = mcp_core::encode_entry(id, &http_entry()).unwrap();
        assert_eq!(
            v["headers"]["Authorization"], "Bearer <需内联:GITHUB_PAT>",
            "{id} 应标记需内联"
        );
        // 解码应还原为 EnvRef，保证可往返
        let back = mcp_core::decode_entry(id, "github", &v).unwrap();
        assert_eq!(
            back.auth.as_ref().and_then(|a| a.env_name()),
            Some("GITHUB_PAT")
        );
    }
}

#[test]
fn literal_token_round_trip() {
    let mut e = http_entry();
    e.auth = Some(Auth::Bearer {
        token: SecretSource::Literal("ghp_plain".into()),
    });
    for a in adapters::all() {
        let v = a.encode_entry(&e).unwrap();
        let back = a.decode_entry("github", &v).unwrap();
        match back.auth {
            Some(Auth::Bearer {
                token: SecretSource::Literal(v),
            }) => {
                assert_eq!(v, "ghp_plain", "{}", a.id())
            }
            other => panic!("{} 明文 token 往返失败：{other:?}", a.id()),
        }
    }
}

// ---------------------------------------------------------------- 方言细节

#[test]
fn opencode_v2_shape() {
    let a = adapters::by_id("opencode").unwrap();
    assert_eq!(a.root_key(Scope::User), vec!["mcp", "servers"]);

    let v = a.encode_entry(&stdio_entry()).unwrap();
    assert_eq!(v["type"], "local");
    assert_eq!(v["command"][0], "python", "command 必须是数组");
    assert_eq!(v["command"][1], "-u");
    assert_eq!(
        v["environment"]["DB_URL"], "postgres://x",
        "env 键应为 environment"
    );
    assert!(v.get("env").is_none());
    assert!(v.get("disabled").is_none(), "启用时不应写 disabled");

    let mut off = stdio_entry();
    off.enabled = false;
    let v2 = a.encode_entry(&off).unwrap();
    assert_eq!(v2["disabled"], true, "禁用时应写 disabled: true");
    let back = a.decode_entry("stockdb", &v2).unwrap();
    assert!(!back.enabled);
}

#[test]
fn antigravity_uses_server_url() {
    let v = mcp_core::encode_entry("antigravity", &http_entry()).unwrap();
    assert_eq!(v["serverUrl"], "https://api.githubcopilot.com/mcp/");
    assert!(v.get("url").is_none(), "Antigravity 不支持 url 键");
}

#[test]
fn windsurf_uses_server_url() {
    let v = mcp_core::encode_entry("windsurf", &http_entry()).unwrap();
    assert_eq!(v["serverUrl"], "https://api.githubcopilot.com/mcp/");
}

#[test]
fn vscode_root_key_depends_on_scope() {
    let a = adapters::by_id("vscode").unwrap();
    assert_eq!(a.root_key(Scope::User), vec!["servers"]);
    assert_eq!(a.root_key(Scope::Project), vec!["mcpServers"]);
    assert!(!a.supports_scope(Scope::Local));
}

#[test]
fn scope_support_matrix() {
    let expect: &[(&str, &[Scope])] = &[
        ("claude-code", &[Scope::User, Scope::Project, Scope::Local]),
        ("codex", &[Scope::User, Scope::Project]),
        ("kimi-code", &[Scope::User]),
        ("workbuddy", &[Scope::User]),
        ("windsurf", &[Scope::User]),
        ("trae", &[Scope::User, Scope::Project]),
        ("claude-desktop", &[Scope::User]),
    ];
    for (id, scopes) in expect {
        let a = adapters::by_id(id).unwrap();
        for s in [Scope::User, Scope::Project, Scope::Local] {
            assert_eq!(a.supports_scope(s), scopes.contains(&s), "{id} / {s:?}");
        }
    }
}

#[test]
fn extra_headers_preserved() {
    let mut e = http_entry();
    e.headers.insert("X-Team".into(), "platform".into());
    for a in adapters::all() {
        let v = a.encode_entry(&e).unwrap();
        let back = a.decode_entry("github", &v).unwrap();
        assert_eq!(
            back.headers.get("X-Team").map(String::as_str),
            Some("platform"),
            "{}",
            a.id()
        );
        assert!(
            !back
                .headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("authorization")),
            "{} Authorization 应被归一为 auth 字段",
            a.id()
        );
    }
}

#[test]
fn codex_toml_dialect_fields() {
    let mut e = http_entry();
    e.timeout_seconds = Some(30);
    let v = mcp_core::encode_entry("codex", &e).unwrap();
    assert_eq!(v["startup_timeout_sec"], 30);
    assert_eq!(v["enabled"], true);
}

#[test]
fn sse_transport_kept() {
    let mut e = http_entry();
    e.transport = Transport::Sse;
    for a in adapters::all() {
        let v = a.encode_entry(&e).unwrap();
        let back = a.decode_entry("github", &v).unwrap();
        // 只有 type 值能区分 http/sse 的客户端才要求保留；
        // OpenCode v2 统一用 "remote"，Antigravity 无 type 字段 —— 降级为 http 是可接受的
        let spec = a.spec();
        if spec.type_field.is_some() && spec.remote_type_value.is_none() {
            assert_eq!(back.transport, Transport::Sse, "{} 应保留 sse", a.id());
        }
    }
}

#[test]
fn env_order_stable() {
    let mut e = stdio_entry();
    e.env = BTreeMap::from([
        ("B".to_string(), "2".to_string()),
        ("A".to_string(), "1".to_string()),
    ]);
    let v = mcp_core::encode_entry("claude-code", &e).unwrap();
    let keys: Vec<&String> = v["env"].as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["A", "B"], "BTreeMap 保证顺序稳定");
}
