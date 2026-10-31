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
    Value::Array(
        values
            .iter()
            .zip(argument_roles(values))
            .map(|(value, role)| redact(value, role.sensitive_value))
            .collect(),
    )
}

struct ArgumentRole {
    sensitive_value: bool,
    inline_option: bool,
}

// A consumed credential is never interpreted again as an option or option terminator.
fn argument_roles(values: &[Value]) -> Vec<ArgumentRole> {
    let mut next_sensitive = false;
    let mut positional = false;
    values
        .iter()
        .map(|value| {
            let sensitive_value = next_sensitive;
            next_sensitive = false;
            let mut inline_option = false;
            if let Some(arg) = value.as_str() {
                if !sensitive_value && arg == "--" {
                    positional = true;
                }
                if !sensitive_value && !positional && arg.starts_with('-') {
                    inline_option = arg.contains('=');
                    next_sensitive = !inline_option && sensitive_name(arg);
                }
            }
            ArgumentRole {
                sensitive_value,
                inline_option,
            }
        })
        .collect()
}

/// Prefixes of actual inline options. Password values resembling `-value=suffix` have no prefix.
pub fn inline_option_prefixes(values: &[Value]) -> Vec<Option<String>> {
    values
        .iter()
        .zip(argument_roles(values))
        .map(|(value, role)| {
            role.inline_option
                .then(|| {
                    value
                        .as_str()
                        .unwrap()
                        .split_once('=')
                        .map(|(flag, _)| format!("{flag}="))
                })
                .flatten()
        })
        .collect()
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
    if parts.len() > 1
        && parts
            .last()
            .is_some_and(|p| ["file", "path", "dir", "directory"].contains(p))
    {
        return false;
    }
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
    if !text.contains('\0') {
        return !text.trim().is_empty();
    }
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    let text = lower
        .strip_prefix("bearer ")
        .or_else(|| lower.strip_prefix("basic "))
        .unwrap_or(&lower)
        .trim_matches(|c: char| c.is_whitespace() || c == '\0');
    !text.is_empty()
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

fn url_credentials(text: &str) -> bool {
    // Split raw URL boundaries before decoding: %2F/%3F in a password are not URL separators.
    if text.split("://").skip(1).any(|rest| {
        rest.split(['/', '?', '#', ' ', '\"', '\''])
            .next()
            .and_then(|authority| authority.rsplit_once('@'))
            .is_some_and(|(userinfo, _)| userinfo.chars().any(|c| c != ':' && c != '\0'))
    }) {
        return true;
    }
    text.split(['?', '#']).skip(1).any(|query| {
        query.split(['&', ';']).any(|pair| {
            pair.split_once('=')
                .is_some_and(|(key, value)| sensitive_name(&percent_decode(key)) && literal(value))
        })
    })
}

fn contains_secret(value: &str) -> bool {
    let text = without_references(value);
    if url_credentials(&text) {
        return true;
    }
    let text = percent_decode(&text);
    let lower = text.to_ascii_lowercase();
    if text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----") {
        return true;
    }
    if lower.starts_with("bearer ")
        && !lower[7..]
            .trim_matches(|c: char| c.is_whitespace() || c == '\0')
            .is_empty()
        || lower.starts_with("basic ") && basic_credentials(text[6..].trim())
    {
        return true;
    }
    let mut pending: Option<bool> = None;
    for part in text
        .split(|c: char| c.is_whitespace() || ['\"', '\'', ',', ';', '&', '?', '#'].contains(&c))
        .filter(|s| !s.is_empty())
    {
        if let Some(allow_scheme) = pending.take() {
            if allow_scheme && auth_scheme(part) {
                pending = Some(false);
                continue;
            }
            if literal(part) {
                return true;
            }
            continue;
        }
        let mut component = part;
        loop {
            if key_shape(component) {
                return true;
            }
            if let Some(separator) = component.find(['=', ':']) {
                let name = &component[..separator];
                let data = &component[separator + 1..];
                if name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
                    && sensitive_name(name)
                {
                    let authorization = authorization_name(name);
                    if data.is_empty() {
                        if component.as_bytes()[separator] == b':' {
                            pending = Some(authorization);
                        }
                    } else if authorization && auth_scheme(data) {
                        pending = Some(false);
                    } else if literal(data) {
                        return true;
                    }
                    break;
                }
                component = data;
                if component.is_empty() {
                    break;
                }
            } else {
                if component.starts_with('-') && sensitive_name(component) {
                    pending = Some(authorization_name(component));
                }
                break;
            }
        }
    }
    false
}

fn auth_scheme(value: &str) -> bool {
    value.eq_ignore_ascii_case("bearer") || value.eq_ignore_ascii_case("basic")
}

fn authorization_name(name: &str) -> bool {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
        .ends_with("authorization")
}

fn key_shape(value: &str) -> bool {
    (value.len() >= 24
        && ["sk-", "ghp_", "gho_", "github_pat_", "xoxb-", "xoxp-"]
            .iter()
            .any(|prefix| value.starts_with(*prefix)))
        || (value.starts_with("eyJ") && value.split('.').count() == 3 && value.len() >= 32)
}

// Recognize encoded user:password instead of treating ordinary text such as "Basic mode" as a secret.
fn basic_credentials(value: &str) -> bool {
    if value.len() < 4 || value.len() % 4 != 0 {
        return false;
    }
    let payload = value.trim_end_matches('=');
    if value.len() - payload.len() > 2
        || !payload
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    {
        return false;
    }
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in payload.bytes() {
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return false,
        };
        buffer = (buffer << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            if (buffer >> bits) & 0xff == u32::from(b':') {
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
