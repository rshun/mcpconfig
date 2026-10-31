//! 按字段和参数语义隐藏凭据，普通配置保留原值。原始凭据只用于实际写盘。
use serde_json::Value;

const HIDDEN: &str = "[已隐藏]";

pub fn config_entry(value: &Value) -> Value {
    redact(value, false)
}

fn redact(value: &Value, sensitive: bool) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let safe = if !sensitive && key == "auth" && value.is_object() {
                        redact(value, true)
                    } else if sensitive
                        && key == "type"
                        && value
                            .as_str()
                            .is_some_and(|s| ["bearer", "basic"].contains(&s))
                    {
                        value.clone()
                    } else if !sensitive
                        && ["args", "command"].contains(&key.as_str())
                        && value.is_array()
                    {
                        arguments(value.as_array().unwrap())
                    } else if ["token_env", "bearer_token_env_var", "tokenEnv"]
                        .contains(&key.as_str())
                        && value.as_str().is_some_and(env_name)
                    {
                        value.clone()
                    } else {
                        redact(value, sensitive || sensitive_name(key))
                    };
                    (key.clone(), safe)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(|v| redact(v, sensitive)).collect()),
        Value::String(text) if (sensitive && literal(text)) || contains_secret(text) => {
            Value::String(HIDDEN.into())
        }
        Value::String(_) | Value::Null => value.clone(),
        _ if sensitive => Value::String(HIDDEN.into()),
        _ => value.clone(),
    }
}

fn arguments(values: &[Value]) -> Value {
    let mut next_sensitive = false;
    let mut positional = false;
    Value::Array(
        values
            .iter()
            .map(|value| {
                let shown = redact(value, next_sensitive);
                next_sensitive = false;
                if let Some(arg) = value.as_str() {
                    if arg == "--" {
                        positional = true;
                    }
                    if !positional
                        && arg.starts_with('-')
                        && !arg.contains('=')
                        && sensitive_name(arg)
                    {
                        next_sensitive = true;
                    }
                }
                shown
            })
            .collect(),
    )
}

fn sensitive_name(name: &str) -> bool {
    let mut words = String::new();
    let mut lower = false;
    for ch in name.chars() {
        if ch.is_ascii_uppercase() && lower {
            words.push(' ');
        }
        words.push(if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            ' '
        });
        lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }
    let compact: String = words.chars().filter(|c| !c.is_whitespace()).collect();
    let parts: Vec<_> = words.split_whitespace().collect();
    parts.iter().any(|p| {
        [
            "password",
            "passwd",
            "pwd",
            "secret",
            "token",
            "credential",
            "credentials",
            "authorization",
            "cookie",
            "pin",
            "pat",
        ]
        .contains(p)
    }) || [
        "apikey",
        "accesskey",
        "privatekey",
        "secret",
        "password",
        "token",
        "cookie",
        "signature",
        "sessionid",
        "connectionstring",
    ]
    .iter()
    .any(|suffix| compact.ends_with(*suffix))
        || ["key", "auth", "bearer", "signature", "sig"].contains(&compact.as_str())
}

fn env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// Mark valid variable references without resolving them. The marker also consumes a CLI value.
fn without_references(value: &str) -> String {
    let mut rest = value;
    let mut text = String::new();
    while !rest.is_empty() {
        let prefix = ["${ENV:", "${env:", "{env:", "${"]
            .into_iter()
            .find(|p| rest.starts_with(*p));
        if let Some(prefix) = prefix {
            if let Some(end) = rest.find('}') {
                if env_name(&rest[prefix.len()..end]) {
                    text.push('\0');
                } else {
                    text.push_str(&rest[..=end]);
                }
                rest = &rest[end + 1..];
                continue;
            }
            text.push_str(rest);
            break;
        }
        let ch = rest.chars().next().unwrap();
        text.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    text
}

fn literal(value: &str) -> bool {
    let text = without_references(value);
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    let lower = text.to_ascii_lowercase();
    let text = lower
        .strip_prefix("bearer ")
        .or_else(|| lower.strip_prefix("basic "))
        .unwrap_or(&lower)
        .trim_matches(|c: char| c.is_whitespace() || c == '\0');
    !text.is_empty() && !["bearer", "basic"].contains(&text)
}

fn percent_decode(value: &str) -> String {
    let mut out = Vec::new();
    let mut bytes = value.as_bytes().iter().copied();
    while let Some(ch) = bytes.next() {
        if ch == b'%' {
            let pair = [bytes.next(), bytes.next()];
            if let [Some(a), Some(b)] = pair {
                if let (Some(a), Some(b)) = ((a as char).to_digit(16), (b as char).to_digit(16)) {
                    out.push((a * 16 + b) as u8);
                    continue;
                }
            }
            out.push(ch);
            out.extend(pair.into_iter().flatten());
        } else {
            out.push(ch);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn contains_secret(value: &str) -> bool {
    let text = without_references(value);
    let text = percent_decode(&text);
    let lower = text.to_ascii_lowercase();
    if text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----") {
        return true;
    }
    // Userinfo is authentication material. Keep credential-free URLs, including their query.
    if text.split("://").skip(1).any(|rest| {
        rest.split(['/', '?', '#', ' ', '\"', '\''])
            .next()
            .and_then(|authority| authority.rsplit_once('@'))
            .is_some_and(|(userinfo, _)| userinfo.chars().any(|c| c != ':' && c != '\0'))
    }) {
        return true;
    }
    if ["bearer ", "basic "].iter().any(|prefix| {
        lower.starts_with(*prefix)
            && !lower[prefix.len()..]
                .trim_matches(|c: char| c.is_whitespace() || c == '\0')
                .is_empty()
    }) {
        return true;
    }
    let mut next_sensitive = false;
    for part in text
        .split(|c: char| c.is_whitespace() || ['\"', '\'', ',', ';', '&', '?', '#'].contains(&c))
        .filter(|s| !s.is_empty())
    {
        for component in part.split(['=', ':']).filter(|s| !s.is_empty()) {
            if component == "\0" {
                next_sensitive = false;
                continue;
            }
            if next_sensitive {
                if component.eq_ignore_ascii_case("bearer")
                    || component.eq_ignore_ascii_case("basic")
                {
                    continue;
                }
                return true;
            }
            if (part.contains(['=', ':']) || component.starts_with('-'))
                && sensitive_name(component)
            {
                next_sensitive = true;
            }
            if component.len() >= 24
                && ["sk-", "ghp_", "gho_", "github_pat_", "xoxb-", "xoxp-"]
                    .iter()
                    .any(|prefix| component.starts_with(*prefix))
            {
                return true;
            }
            if component.starts_with("eyJ")
                && component.split('.').count() == 3
                && component.len() >= 32
            {
                return true;
            }
        }
    }
    false
}

/// 用不同的固定标记表示敏感值发生变化，既不泄露值/摘要，也不把变更隐藏成“无变化”。
pub fn change(before: Option<&Value>, after: &Value) -> (Option<Value>, Value) {
    let mut shown_before = before.map(config_entry);
    let mut shown_after = config_entry(after);
    if let (Some(before), Some(shown_before)) = (before, &mut shown_before) {
        mark_changes(before, after, shown_before, &mut shown_after);
    }
    (shown_before, shown_after)
}

fn mark_changes(before: &Value, after: &Value, old: &mut Value, new: &mut Value) {
    match (before, after) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, value) in a {
                if let (Some(other), Some(old), Some(new)) =
                    (b.get(key), old.get_mut(key), new.get_mut(key))
                {
                    mark_changes(value, other, old, new);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if let (Some(old), Some(new)) = (old.as_array_mut(), new.as_array_mut()) {
                for (((a, b), old), new) in a.iter().zip(b).zip(old).zip(new) {
                    mark_changes(a, b, old, new);
                }
            }
        }
        _ if before != after && old.as_str() == Some(HIDDEN) && new.as_str() == Some(HIDDEN) => {
            *old = Value::String("[已隐藏：原值]".into());
            *new = Value::String("[已隐藏：新值]".into());
        }
        _ => {}
    }
}
