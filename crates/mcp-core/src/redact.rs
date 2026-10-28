//! 展示层统一脱敏。原始值仅用于编码、写盘，不进入 diff 或检查 DTO。
use serde_json::Value;

const HIDDEN: &str = "[已隐藏]";

pub fn config_entry(value: &Value) -> Value {
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
        _ => Value::String(HIDDEN.into()),
    }
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
