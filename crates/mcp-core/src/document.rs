//! 配置文档模型：**保留未知字段**地增量修改 JSON / JSONC / TOML。
//!
//! 关键约束：只改目标子树，其余内容（其他键、其他表、注释、格式）原样保留。
//! 本模块不做任何文件 IO。

use crate::adapters::Format;
use crate::error::{Error, Result};
use serde_json::Map as JsonMap;
use serde_json::Value;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue};

/// 同名条目的覆盖方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergeMode {
    /// 字段级合并：清单声明了的字段覆盖，没声明的保留文件原值
    #[default]
    Merge,
    /// 整条替换：以清单为准，文件里多出的字段会被删除
    Replace,
}

/// 单条目变更类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    New,
    Update,
    Unchanged,
}

impl ChangeKind {
    pub fn label(self) -> &'static str {
        match self {
            ChangeKind::New => "新增",
            ChangeKind::Update => "修改",
            ChangeKind::Unchanged => "无变化",
        }
    }
}

/// 配置文档。
pub enum ConfigDoc {
    Json(Value),
    Toml(DocumentMut),
    TomlNew,
    JsonNew,
}

impl ConfigDoc {
    /// 解析文本。空文本视为空文档。
    pub fn parse(format: Format, text: &str) -> Result<Self> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(match format {
                Format::Toml => ConfigDoc::Toml(DocumentMut::new()),
                _ => ConfigDoc::Json(Value::Object(JsonMap::new())),
            });
        }
        match format {
            Format::Json | Format::Jsonc => {
                let cleaned = strip_jsonc(trimmed);
                let v: Value = serde_json::from_str(&cleaned)
                    .map_err(|e| Error::Parse(format!("JSON 解析失败：{e}")))?;
                if !v.is_object() {
                    return Err(Error::Parse("配置文件根节点不是对象".into()));
                }
                Ok(ConfigDoc::Json(v))
            }
            Format::Toml => trimmed
                .parse::<DocumentMut>()
                .map(ConfigDoc::Toml)
                .map_err(|e| Error::Parse(format!("TOML 解析失败：{e}"))),
        }
    }

    pub fn is_json(&self) -> bool {
        matches!(self, ConfigDoc::Json(_))
    }

    fn json_root_mut(&mut self, root: &[String]) -> Result<&mut JsonMap<String, Value>> {
        match self {
            ConfigDoc::Json(v) => {
                let mut cur = v;
                for seg in root {
                    if !cur.is_object() {
                        return Err(Error::Parse(format!("根键 {seg} 的父节点不是对象")));
                    }
                    let obj = cur.as_object_mut().unwrap();
                    if !obj.contains_key(seg) {
                        obj.insert(seg.clone(), Value::Object(JsonMap::new()));
                    }
                    cur = obj.get_mut(seg).unwrap();
                }
                cur.as_object_mut()
                    .ok_or_else(|| Error::Parse("根键不是对象".into()))
            }
            _ => Err(Error::Parse("不是 JSON 文档".into())),
        }
    }

    fn toml_root_mut(&mut self, root: &[String]) -> Result<&mut Table> {
        match self {
            ConfigDoc::Toml(doc) => {
                let mut tbl: &mut Table = doc.as_table_mut();
                for seg in root {
                    if !tbl.contains_key(seg) {
                        // 隐式表：仅作为子表的容器，不单独渲染表头
                        let mut t = Table::new();
                        t.set_implicit(true);
                        tbl.insert(seg, Item::Table(t));
                    }
                    tbl = tbl
                        .get_mut(seg)
                        .and_then(Item::as_table_mut)
                        .ok_or_else(|| Error::Parse(format!("根键 {seg} 不是表")))?;
                }
                Ok(tbl)
            }
            _ => Err(Error::Parse("不是 TOML 文档".into())),
        }
    }

    /// 列出容器下的所有条目名。
    pub fn names(&self, root: &[String]) -> Vec<String> {
        match self {
            ConfigDoc::Json(v) => walk_json(v, root)
                .and_then(Value::as_object)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default(),
            ConfigDoc::Toml(doc) => walk_toml(doc.as_table(), root)
                .map(|t| t.iter().map(|(k, _)| k.to_string()).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// 读取单条目（统一为 JSON 表示）。
    pub fn get(&self, root: &[String], name: &str) -> Option<Value> {
        match self {
            ConfigDoc::Json(v) => walk_json(v, root)
                .and_then(Value::as_object)
                .and_then(|m| m.get(name))
                .cloned(),
            ConfigDoc::Toml(doc) => walk_toml(doc.as_table(), root)
                .and_then(|t| t.get(name))
                .map(item_to_json),
            _ => None,
        }
    }

    /// 写入单条目，返回变更类型。**内容一致时不做任何修改**（幂等）。
    pub fn upsert(
        &mut self,
        root: &[String],
        name: &str,
        incoming: &Value,
        mode: MergeMode,
    ) -> Result<ChangeKind> {
        let existing = self.get(root, name);
        let (value, kind) = match (existing, mode) {
            (None, _) => (incoming.clone(), ChangeKind::New),
            (Some(old), MergeMode::Replace) => {
                if &old == incoming {
                    return Ok(ChangeKind::Unchanged);
                }
                (incoming.clone(), ChangeKind::Update)
            }
            (Some(old), MergeMode::Merge) => {
                let merged = merge_json(&old, incoming);
                if merged == old {
                    return Ok(ChangeKind::Unchanged);
                }
                (merged, ChangeKind::Update)
            }
        };

        match self {
            ConfigDoc::Json(_) => {
                let map = self.json_root_mut(root)?;
                map.insert(name.to_string(), value);
            }
            ConfigDoc::Toml(_) => {
                let tbl = self.toml_root_mut(root)?;
                tbl.insert(name, json_to_toml_table(&value));
            }
            _ => return Err(Error::Parse("文档未初始化".into())),
        }
        Ok(kind)
    }

    /// 删除单条目，返回是否删除成功。
    pub fn remove(&mut self, root: &[String], name: &str) -> Result<bool> {
        match self {
            ConfigDoc::Json(_) => {
                let map = self.json_root_mut(root)?;
                Ok(map.remove(name).is_some())
            }
            ConfigDoc::Toml(_) => {
                let tbl = self.toml_root_mut(root)?;
                Ok(tbl.remove(name).is_some())
            }
            _ => Ok(false),
        }
    }

    /// 渲染为文本（保留原有格式与注释）。
    pub fn render(&self) -> String {
        match self {
            ConfigDoc::Json(v) => {
                let mut s = serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".into());
                s.push('\n');
                s
            }
            ConfigDoc::Toml(doc) => doc.to_string(),
            _ => String::new(),
        }
    }
}

// ---------------------------------------------------------------- JSON 路径

fn walk_json<'a>(v: &'a Value, root: &[String]) -> Option<&'a Value> {
    let mut cur = v;
    for seg in root {
        cur = cur.as_object()?.get(seg)?;
    }
    Some(cur)
}

// ---------------------------------------------------------------- TOML 路径

fn walk_toml<'a>(tbl: &'a Table, root: &[String]) -> Option<&'a Table> {
    let mut cur = tbl;
    for seg in root {
        cur = cur.get(seg)?.as_table()?;
    }
    Some(cur)
}

// ---------------------------------------------------------------- 合并

/// 字段级合并：对象递归合并，其余类型以 incoming 覆盖。
pub fn merge_json(base: &Value, incoming: &Value) -> Value {
    match (base, incoming) {
        (Value::Object(a), Value::Object(b)) => {
            let mut out = a.clone();
            for (k, v) in b {
                let merged = match out.get(k) {
                    Some(existing) => merge_json(existing, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        _ => incoming.clone(),
    }
}

// ---------------------------------------------------------------- TOML ↔ JSON

fn item_to_json(item: &Item) -> Value {
    match item {
        Item::Value(v) => toml_value_to_json(v),
        Item::Table(t) => {
            let mut m = JsonMap::new();
            for (k, v) in t.iter() {
                m.insert(k.to_string(), item_to_json(v));
            }
            Value::Object(m)
        }
        Item::ArrayOfTables(arr) => Value::Array(arr.iter().map(table_to_json).collect()),
        Item::None => Value::Null,
    }
}

fn table_to_json(t: &Table) -> Value {
    let mut m = JsonMap::new();
    for (k, v) in t.iter() {
        m.insert(k.to_string(), item_to_json(v));
    }
    Value::Object(m)
}

fn toml_value_to_json(v: &TomlValue) -> Value {
    match v {
        TomlValue::String(s) => Value::String(s.value().clone()),
        TomlValue::Integer(i) => Value::from(*i.value()),
        TomlValue::Float(f) => serde_json::Number::from_f64(*f.value())
            .map(Value::Number)
            .unwrap_or(Value::Null),
        TomlValue::Boolean(b) => Value::Bool(*b.value()),
        TomlValue::Datetime(d) => Value::String(d.value().to_string()),
        TomlValue::Array(a) => Value::Array(a.iter().map(toml_value_to_json).collect()),
        TomlValue::InlineTable(t) => {
            let mut m = JsonMap::new();
            for (k, val) in t.iter() {
                m.insert(k.to_string(), toml_value_to_json(val));
            }
            Value::Object(m)
        }
    }
}

/// 单条目渲染为 TOML 表：嵌套对象用内联表，避免生成过多子表段。
fn json_to_toml_table(v: &Value) -> Item {
    let mut tbl = Table::new();
    if let Value::Object(m) = v {
        for (k, val) in m {
            let item = match val {
                Value::Object(_) => Item::Value(TomlValue::InlineTable(json_to_inline(val))),
                Value::Array(arr) => {
                    let mut a = Array::new();
                    for e in arr {
                        if let Some(tv) = json_to_toml_value(e) {
                            a.push_formatted(tv);
                        }
                    }
                    Item::Value(TomlValue::Array(a))
                }
                other => match json_to_toml_value(other) {
                    Some(tv) => Item::Value(tv),
                    None => continue,
                },
            };
            tbl.insert(k, item);
        }
    }
    Item::Table(tbl)
}

fn json_to_inline(v: &Value) -> InlineTable {
    let mut t = InlineTable::new();
    if let Value::Object(m) = v {
        for (k, val) in m {
            if let Some(tv) = json_to_toml_value(val) {
                t.insert(k, tv);
            }
        }
    }
    t
}

fn json_to_toml_value(v: &Value) -> Option<TomlValue> {
    Some(match v {
        Value::String(s) => TomlValue::from(s.clone()),
        Value::Bool(b) => TomlValue::from(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                TomlValue::from(i)
            } else {
                TomlValue::from(n.as_f64()?)
            }
        }
        Value::Array(arr) => {
            let mut a = Array::new();
            for e in arr {
                a.push_formatted(json_to_toml_value(e)?);
            }
            TomlValue::Array(a)
        }
        Value::Object(_) => TomlValue::InlineTable(json_to_inline(v)),
        Value::Null => return None,
    })
}

// ---------------------------------------------------------------- JSONC

/// 去掉 JSONC 的注释与尾逗号（正确跳过字符串内部）。
pub fn strip_jsonc(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut in_str = false;
    let mut esc = false;

    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            out.push(c as char);
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_str = true;
                out.push('"');
                i += 1;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            _ => {
                out.push(c as char);
                i += 1;
            }
        }
    }
    strip_trailing_commas(&out)
}

fn strip_trailing_commas(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut in_str = false;
    let mut esc = false;

    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            out.push(c as char);
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
            out.push('"');
            i += 1;
            continue;
        }
        if c == b',' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'}' || bytes[j] == b']') {
                i += 1; // 丢弃该逗号
                continue;
            }
        }
        out.push(c as char);
        i += 1;
    }
    out
}

/// 检测文本是否含注释或尾逗号（JSONC 特征）。
pub fn has_jsonc_features(src: &str) -> bool {
    let stripped_comments = {
        let bytes = src.as_bytes();
        let mut i = 0;
        let mut in_str = false;
        let mut esc = false;
        let mut found = false;
        while i < bytes.len() {
            let c = bytes[i];
            if in_str {
                if esc {
                    esc = false;
                } else if c == b'\\' {
                    esc = true;
                } else if c == b'"' {
                    in_str = false;
                }
                i += 1;
                continue;
            }
            if c == b'"' {
                in_str = true;
                i += 1;
                continue;
            }
            if c == b'/' && i + 1 < bytes.len() && (bytes[i + 1] == b'/' || bytes[i + 1] == b'*') {
                found = true;
                break;
            }
            i += 1;
        }
        found
    };
    stripped_comments || strip_trailing_commas(src) != src
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn root(k: &str) -> Vec<String> {
        vec![k.to_string()]
    }

    #[test]
    fn json_preserves_unknown_keys() {
        let mut doc = ConfigDoc::parse(
            Format::Json,
            r#"{"mcpServers":{"a":{"command":"npx"}},"other":{"keep":1}}"#,
        )
        .unwrap();
        let r = root("mcpServers");
        let kind = doc
            .upsert(
                &r,
                "b",
                &json!({"command":"node","args":["x.js"]}),
                MergeMode::Merge,
            )
            .unwrap();
        assert_eq!(kind, ChangeKind::New);

        let out = doc.render();
        assert!(out.contains("\"other\""), "未知顶层键必须保留");
        assert!(out.contains("\"a\""), "其他条目必须保留");
        assert!(out.contains("\"b\""));
    }

    #[test]
    fn json_merge_keeps_undeclared_fields() {
        let mut doc = ConfigDoc::parse(
            Format::Json,
            r#"{"mcpServers":{"github":{"type":"http","url":"old","cwd":"/w"}}}"#,
        )
        .unwrap();
        let r = root("mcpServers");
        let kind = doc
            .upsert(
                &r,
                "github",
                &json!({"type":"http","url":"new"}),
                MergeMode::Merge,
            )
            .unwrap();
        assert_eq!(kind, ChangeKind::Update);

        let v = doc.get(&r, "github").unwrap();
        assert_eq!(v["url"], "new");
        assert_eq!(v["cwd"], "/w", "清单未声明的字段必须保留");
    }

    #[test]
    fn json_replace_drops_extra_fields() {
        let mut doc = ConfigDoc::parse(
            Format::Json,
            r#"{"mcpServers":{"github":{"url":"old","cwd":"/w"}}}"#,
        )
        .unwrap();
        let r = root("mcpServers");
        doc.upsert(&r, "github", &json!({"url":"new"}), MergeMode::Replace)
            .unwrap();
        let v = doc.get(&r, "github").unwrap();
        assert_eq!(v["url"], "new");
        assert!(v.get("cwd").is_none(), "replace 模式应删除多余字段");
    }

    #[test]
    fn identical_content_is_unchanged() {
        let mut doc =
            ConfigDoc::parse(Format::Json, r#"{"mcpServers":{"a":{"command":"npx"}}}"#).unwrap();
        let r = root("mcpServers");
        let kind = doc
            .upsert(&r, "a", &json!({"command":"npx"}), MergeMode::Merge)
            .unwrap();
        assert_eq!(kind, ChangeKind::Unchanged, "幂等：一致时不应改写");
    }

    #[test]
    fn toml_preserves_comments_and_other_tables() {
        let src = r#"# 顶部注释
[plugins.browser]
enabled = true

[mcp_servers.old]
command = "npx"
"#;
        let mut doc = ConfigDoc::parse(Format::Toml, src).unwrap();
        let r = root("mcp_servers");
        doc.upsert(
            &r,
            "new",
            &json!({"command":"node","args":["x.js"],"env":{"K":"V"}}),
            MergeMode::Merge,
        )
        .unwrap();

        let out = doc.render();
        assert!(out.contains("# 顶部注释"), "注释必须保留");
        assert!(out.contains("[plugins.browser]"), "无关表必须保留");
        assert!(out.contains("[mcp_servers.old]"), "其他条目必须保留");
        assert!(out.contains("[mcp_servers.new]"));
        assert!(out.contains("env = { K = \"V\" }"), "嵌套对象应为内联表");
    }

    #[test]
    fn toml_round_trips_json_view() {
        let doc = ConfigDoc::parse(
            Format::Toml,
            "[mcp_servers.x]\ncommand = \"npx\"\nargs = [\"-y\", \"pkg\"]\nenabled = true\n",
        )
        .unwrap();
        let v = doc.get(&root("mcp_servers"), "x").unwrap();
        assert_eq!(v["command"], "npx");
        assert_eq!(v["args"][1], "pkg");
        assert_eq!(v["enabled"], true);
    }

    #[test]
    fn creates_nested_root_when_absent() {
        let mut doc = ConfigDoc::parse(Format::Json, r#"{}"#).unwrap();
        let r = vec!["mcp".to_string(), "servers".to_string()];
        doc.upsert(&r, "x", &json!({"type":"local"}), MergeMode::Merge)
            .unwrap();
        let out = doc.render();
        assert!(out.contains("\"mcp\""));
        assert!(out.contains("\"servers\""));
    }

    #[test]
    fn jsonc_strip_handles_strings() {
        let src = r#"{
  // 行注释
  "a": "http://x//y",
  /* 块注释 */
  "b": [1, 2,],
}"#;
        let cleaned = strip_jsonc(src);
        let v: Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(v["a"], "http://x//y", "字符串内的 // 不能被当作注释");
        assert_eq!(v["b"][0], 1);
    }

    #[test]
    fn detects_jsonc_features() {
        assert!(has_jsonc_features("{\n// c\n\"a\":1}"));
        assert!(has_jsonc_features("{\"a\":1,}"));
        assert!(!has_jsonc_features("{\"a\":1}"));
    }

    #[test]
    fn remove_entry() {
        let mut doc = ConfigDoc::parse(Format::Json, r#"{"mcpServers":{"a":{},"b":{}}}"#).unwrap();
        let r = root("mcpServers");
        assert!(doc.remove(&r, "a").unwrap());
        assert!(doc.get(&r, "a").is_none());
        assert!(doc.get(&r, "b").is_some());
    }

    #[test]
    fn empty_text_is_empty_doc() {
        let doc = ConfigDoc::parse(Format::Toml, "   ").unwrap();
        assert!(doc.names(&root("mcp_servers")).is_empty());
    }

    #[test]
    fn names_lists_entries() {
        let doc = ConfigDoc::parse(Format::Json, r#"{"mcpServers":{"a":{},"b":{}}}"#).unwrap();
        let mut n = doc.names(&root("mcpServers"));
        n.sort();
        assert_eq!(n, vec!["a", "b"]);
    }
}
