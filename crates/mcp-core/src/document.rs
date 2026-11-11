//! 配置文档模型：**保留未知字段、注释与缩进**地增量修改 JSON / JSONC / TOML / YAML。
//!
//! 关键约束：
//! - JSON / JSONC：采用零依赖词法区间补丁（Surgical Span Patching），只替换/插入目标条目区间，
//!   完整保留容器外及兄弟条目的 `//`、`/* */` 注释、尾逗号与文件原有缩进；仅当被覆盖的目标条目值
//!   **内部自身**含注释时标记 `lost_comments = true`。
//! - TOML：基于 `toml_edit` CST 原位编辑，保留注释与其他表。
//! - Merge：支持零密钥键骨架 `ManagedKeys` 驱动的三方合并（3-Way Merge），可准确删除先前由
//!   MCP Hub 写入但已从清单移除的 `env` / `headers` 键，同时保留用户手写字段。
//!
//! 本模块不做任何文件 IO。

use crate::adapters::Format;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Map as JsonMap;
use serde_json::Value;
use std::collections::BTreeMap;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue};

/// 同名条目的覆盖方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergeMode {
    /// 字段级合并：清单声明了的字段覆盖，曾由本工具托管后被移除的键清理，用户手写字段保留
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

/// 零密钥已托管键骨架：仅记录上次由 MCP Hub 写入某条目的键层级树，绝不存储任何标量或凭据值。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedKeys {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, ManagedKeys>,
}

impl ManagedKeys {
    /// 从编码后的客户端 JSON 值提取纯键名骨架（丢弃全部字符串/数值/布尔/数组内容）。
    pub fn from_value(value: &Value) -> Self {
        match value {
            Value::Object(map) => Self {
                keys: map
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::from_value(v)))
                    .collect(),
            },
            _ => Self::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// JSON/JSONC 文档状态（同时维护逻辑 JSON 视图与原始文本区间补丁）。
pub struct JsonDoc {
    value: Value,
    raw: Option<String>,
    lost_comments: bool,
}

/// 配置文档。
pub enum ConfigDoc {
    Json(JsonDoc),
    Toml(DocumentMut),
    Yaml(Value),
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
                Format::Yaml => ConfigDoc::Yaml(Value::Object(JsonMap::new())),
                Format::Json | Format::Jsonc => ConfigDoc::Json(JsonDoc {
                    value: Value::Object(JsonMap::new()),
                    raw: None,
                    lost_comments: false,
                }),
            });
        }
        match format {
            Format::Json | Format::Jsonc => {
                let cleaned = strip_jsonc(trimmed);
                let v: Value = serde_json::from_str(&cleaned).map_err(|e| {
                    Error::Parse(format!(
                        "JSON 解析失败（行 {}，列 {}）",
                        e.line(),
                        e.column()
                    ))
                })?;
                if !v.is_object() {
                    return Err(Error::Parse("配置文件根节点不是对象".into()));
                }
                Ok(ConfigDoc::Json(JsonDoc {
                    value: v,
                    raw: Some(text.to_string()),
                    lost_comments: false,
                }))
            }
            Format::Toml => trimmed
                .parse::<DocumentMut>()
                .map(ConfigDoc::Toml)
                .map_err(|_| Error::Parse("TOML 解析失败，请检查语法".into())),
            Format::Yaml => {
                let v: Value = serde_yaml::from_str(trimmed)
                    .map_err(|_| Error::Parse("YAML 解析失败，请检查语法".into()))?;
                if !v.is_object() {
                    return Err(Error::Parse("配置文件根节点不是对象".into()));
                }
                Ok(ConfigDoc::Yaml(v))
            }
        }
    }

    pub fn is_json(&self) -> bool {
        matches!(self, ConfigDoc::Json(_))
    }

    /// 本次编辑是否因覆盖/删除含内部注释的条目而实际丢失了注释。
    pub fn lost_comments(&self) -> bool {
        match self {
            ConfigDoc::Json(doc) => doc.lost_comments,
            _ => false,
        }
    }

    fn value_root_mut<'a>(v: &'a mut Value, root: &[String]) -> Result<&'a mut JsonMap<String, Value>> {
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
            ConfigDoc::Json(doc) => walk_json(&doc.value, root)
                .and_then(Value::as_object)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default(),
            ConfigDoc::Yaml(v) => walk_json(v, root)
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
            ConfigDoc::Json(doc) => walk_json(&doc.value, root)
                .and_then(Value::as_object)
                .and_then(|m| m.get(name))
                .cloned(),
            ConfigDoc::Yaml(v) => walk_json(v, root)
                .and_then(Value::as_object)
                .and_then(|m| m.get(name))
                .cloned(),
            ConfigDoc::Toml(doc) => walk_toml(doc.as_table(), root)
                .and_then(|t| t.get(name))
                .map(item_to_json),
            _ => None,
        }
    }

    /// 只读提取 MCP 容器，区分缺失与错误结构；支持 TOML 内联表与 YAML。
    pub fn entries(&self, root: &[String]) -> Result<JsonMap<String, Value>> {
        let value = match self {
            ConfigDoc::Json(doc) => doc.value.clone(),
            ConfigDoc::Yaml(v) => v.clone(),
            ConfigDoc::Toml(doc) => table_to_json(doc.as_table()),
            _ => return Ok(JsonMap::new()),
        };
        let mut current = &value;
        for segment in root {
            let object = current
                .as_object()
                .ok_or_else(|| Error::Parse("MCP 容器不是对象".into()))?;
            match object.get(segment) {
                Some(next) => current = next,
                None => return Ok(JsonMap::new()),
            }
        }
        current
            .as_object()
            .cloned()
            .ok_or_else(|| Error::Parse("MCP 容器不是对象".into()))
    }

    /// 写入单条目，返回变更类型。**内容一致时不做任何修改**（幂等）。
    pub fn upsert(
        &mut self,
        root: &[String],
        name: &str,
        incoming: &Value,
        mode: MergeMode,
    ) -> Result<ChangeKind> {
        self.upsert_with_managed(root, name, incoming, mode, None)
    }

    /// 带三方合并键骨架基线的单条目写入。
    pub fn upsert_with_managed(
        &mut self,
        root: &[String],
        name: &str,
        incoming: &Value,
        mode: MergeMode,
        prev_managed: Option<&ManagedKeys>,
    ) -> Result<ChangeKind> {
        let existing = self.get(root, name);
        let (value, kind) = reconcile_entry(existing.as_ref(), incoming, mode, prev_managed);
        if kind == ChangeKind::Unchanged {
            return Ok(ChangeKind::Unchanged);
        }

        match self {
            ConfigDoc::Json(doc) => {
                let map = Self::value_root_mut(&mut doc.value, root)?;
                map.insert(name.to_string(), value.clone());
                if let Some(cur_raw) = doc.raw.clone() {
                    match patch_jsonc_upsert(&cur_raw, root, name, &value) {
                        Some((patched, stripped_interior))
                            if serde_json::from_str::<Value>(&strip_jsonc(patched.trim())).is_ok() =>
                        {
                            doc.raw = Some(patched);
                            if stripped_interior {
                                doc.lost_comments = true;
                            }
                        }
                        _ => {
                            if has_jsonc_comments(&cur_raw) {
                                doc.lost_comments = true;
                            }
                            doc.raw = None;
                        }
                    }
                }
            }
            ConfigDoc::Yaml(v) => {
                let map = Self::value_root_mut(v, root)?;
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
            ConfigDoc::Json(doc) => {
                let map = Self::value_root_mut(&mut doc.value, root)?;
                let removed = map.remove(name).is_some();
                if removed {
                    if let Some(cur_raw) = doc.raw.clone() {
                        match patch_jsonc_remove(&cur_raw, root, name) {
                            Some((patched, stripped_interior))
                                if serde_json::from_str::<Value>(&strip_jsonc(patched.trim()))
                                    .is_ok() =>
                            {
                                doc.raw = Some(patched);
                                if stripped_interior {
                                    doc.lost_comments = true;
                                }
                            }
                            _ => {
                                if has_jsonc_comments(&cur_raw) {
                                    doc.lost_comments = true;
                                }
                                doc.raw = None;
                            }
                        }
                    }
                }
                Ok(removed)
            }
            ConfigDoc::Yaml(v) => {
                let map = Self::value_root_mut(v, root)?;
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
            ConfigDoc::Json(doc) => {
                if let Some(raw) = &doc.raw {
                    raw.clone()
                } else {
                    let mut s =
                        serde_json::to_string_pretty(&doc.value).unwrap_or_else(|_| "{}".into());
                    s.push('\n');
                    s
                }
            }
            ConfigDoc::Yaml(v) => serde_yaml::to_string(v).unwrap_or_default(),
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

// ---------------------------------------------------------------- 二方与三方合并

/// 字段级合并：对象递归合并，其余类型以 incoming 覆盖。
pub fn merge_json(base: &Value, incoming: &Value) -> Value {
    merge_json_3way(base, incoming, None)
}

fn prune_managed(existing: &Value, prev: &ManagedKeys) -> Option<Value> {
    if prev.keys.is_empty() {
        return None;
    }
    let Value::Object(map) = existing else {
        return None;
    };
    let mut out = JsonMap::new();
    for (k, v) in map {
        if let Some(child_prev) = prev.keys.get(k) {
            if let Some(rem) = prune_managed(v, child_prev) {
                out.insert(k.clone(), rem);
            }
        } else {
            out.insert(k.clone(), v.clone());
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(Value::Object(out))
    }
}

/// 三方合并：
/// - `prev_managed` 中存在但 `incoming` 中已移除的键视为「被清单删除」，从目标对象移除；
/// - 从未被 `prev_managed` 托管且不在 `incoming` 中的键视为「用户手写字段」，无损保留；
/// - `incoming` 中声明的键递归覆盖。
pub fn merge_json_3way(
    base: &Value,
    incoming: &Value,
    prev_managed: Option<&ManagedKeys>,
) -> Value {
    match (base, incoming) {
        (Value::Object(a), Value::Object(b)) => {
            let mut out = JsonMap::new();
            for (k, existing_val) in a {
                if b.contains_key(k) {
                    out.insert(k.clone(), existing_val.clone());
                } else if let Some(prev_child) = prev_managed.and_then(|p| p.keys.get(k)) {
                    if let Some(rem) = prune_managed(existing_val, prev_child) {
                        out.insert(k.clone(), rem);
                    }
                } else {
                    out.insert(k.clone(), existing_val.clone());
                }
            }
            for (k, v) in b {
                let next_prev = prev_managed.and_then(|p| p.keys.get(k));
                let merged = match out.get(k) {
                    Some(existing) => merge_json_3way(existing, v, next_prev),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        _ => incoming.clone(),
    }
}

/// 统一条目调和函数（供规划器与 `MatrixView` 漂移检测共同调用，保证判定逻辑严格一致）。
pub fn reconcile_entry(
    existing: Option<&Value>,
    incoming: &Value,
    mode: MergeMode,
    prev_managed: Option<&ManagedKeys>,
) -> (Value, ChangeKind) {
    match (existing, mode) {
        (None, _) => (incoming.clone(), ChangeKind::New),
        (Some(old), MergeMode::Replace) => {
            if old == incoming {
                (old.clone(), ChangeKind::Unchanged)
            } else {
                (incoming.clone(), ChangeKind::Update)
            }
        }
        (Some(old), MergeMode::Merge) => {
            let merged = merge_json_3way(old, incoming, prev_managed);
            if &merged == old {
                (old.clone(), ChangeKind::Unchanged)
            } else {
                (merged, ChangeKind::Update)
            }
        }
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

// ---------------------------------------------------------------- JSONC 清理与区间扫描器

/// 去掉 JSONC 的注释与尾逗号（正确跳过字符串内部）。
pub fn strip_jsonc(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    let mut in_str = false;
    let mut esc = false;

    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            out.push(c);
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
                out.push(b'"');
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
                out.push(c);
                i += 1;
            }
        }
    }
    strip_trailing_commas(&String::from_utf8(out).expect("仅去除 ASCII 注释，不改变 UTF-8"))
}

fn strip_trailing_commas(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    let mut in_str = false;
    let mut esc = false;

    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            out.push(c);
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
            out.push(b'"');
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
        out.push(c);
        i += 1;
    }
    String::from_utf8(out).expect("仅去除 ASCII 尾逗号，不改变 UTF-8")
}

/// 检测指定字节区间内部是否包含 `//` 或 `/*` 注释（跳过字符串内部）。
pub fn has_comment_in_span(src: &str, start: usize, end: usize) -> bool {
    let bytes = &src.as_bytes()[start..end.min(src.len())];
    let mut i = 0;
    let mut in_str = false;
    let mut esc = false;
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
            return true;
        }
        i += 1;
    }
    false
}

pub fn has_jsonc_comments(src: &str) -> bool {
    has_comment_in_span(src, 0, src.len())
}

/// 检测文本是否含注释或尾逗号（JSONC 特征）。
pub fn has_jsonc_features(src: &str) -> bool {
    has_jsonc_comments(src) || strip_trailing_commas(src) != src
}

// ---- 零依赖 JSONC 词法区间扫描器（用于无损局部补丁） ----

#[derive(Debug, Clone)]
struct MemberSpan {
    key: String,
    key_start: usize,
    val_start: usize,
    val_end: usize,
    comma_pos: Option<usize>,
}

#[derive(Debug, Clone)]
struct ObjSpan {
    open_brace: usize,
    close_brace: usize,
    members: Vec<MemberSpan>,
}

struct JsoncScanner<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> JsoncScanner<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            end: src.len(),
        }
    }

    fn new_slice(src: &'a str, start: usize, end: usize) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: start,
            end: end.min(src.len()),
        }
    }

    fn skip_ws_and_comments(&mut self) -> Option<()> {
        while self.pos < self.end {
            let b = self.bytes[self.pos];
            if b.is_ascii_whitespace() {
                self.pos += 1;
                continue;
            }
            // 跳过 UTF-8 BOM
            if self.src[self.pos..self.end].starts_with('\u{feff}') {
                self.pos += '\u{feff}'.len_utf8();
                continue;
            }
            if b == b'/' && self.pos + 1 < self.end {
                if self.bytes[self.pos + 1] == b'/' {
                    self.pos += 2;
                    while self.pos < self.end && self.bytes[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                    continue;
                }
                if self.bytes[self.pos + 1] == b'*' {
                    self.pos += 2;
                    let mut closed = false;
                    while self.pos + 1 < self.end {
                        if self.bytes[self.pos] == b'*' && self.bytes[self.pos + 1] == b'/' {
                            self.pos += 2;
                            closed = true;
                            break;
                        }
                        self.pos += 1;
                    }
                    if !closed {
                        return None;
                    }
                    continue;
                }
            }
            break;
        }
        Some(())
    }

    fn scan_string(&mut self) -> Option<(usize, usize)> {
        if self.pos >= self.end || self.bytes[self.pos] != b'"' {
            return None;
        }
        let start = self.pos;
        self.pos += 1;
        let mut esc = false;
        while self.pos < self.end {
            let b = self.bytes[self.pos];
            self.pos += 1;
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                return Some((start, self.pos));
            }
        }
        None
    }

    fn scan_value(&mut self) -> Option<(usize, usize)> {
        self.skip_ws_and_comments()?;
        if self.pos >= self.end {
            return None;
        }
        let start = self.pos;
        match self.bytes[self.pos] {
            b'"' => self.scan_string(),
            b'{' => {
                let obj = self.parse_object()?;
                Some((obj.open_brace, obj.close_brace + 1))
            }
            b'[' => {
                self.pos += 1;
                loop {
                    self.skip_ws_and_comments()?;
                    if self.pos >= self.end {
                        return None;
                    }
                    if self.bytes[self.pos] == b']' {
                        self.pos += 1;
                        return Some((start, self.pos));
                    }
                    self.scan_value()?;
                    self.skip_ws_and_comments()?;
                    if self.pos < self.end && self.bytes[self.pos] == b',' {
                        self.pos += 1;
                    }
                }
            }
            _ => {
                while self.pos < self.end {
                    let b = self.bytes[self.pos];
                    if b.is_ascii_whitespace()
                        || matches!(b, b',' | b'}' | b']' | b'/')
                    {
                        break;
                    }
                    self.pos += 1;
                }
                if self.pos == start {
                    None
                } else {
                    Some((start, self.pos))
                }
            }
        }
    }

    fn parse_object(&mut self) -> Option<ObjSpan> {
        self.skip_ws_and_comments()?;
        if self.pos >= self.end || self.bytes[self.pos] != b'{' {
            return None;
        }
        let open_brace = self.pos;
        self.pos += 1;
        let mut members = Vec::new();
        loop {
            self.skip_ws_and_comments()?;
            if self.pos >= self.end {
                return None;
            }
            if self.bytes[self.pos] == b'}' {
                let close_brace = self.pos;
                self.pos += 1;
                return Some(ObjSpan {
                    open_brace,
                    close_brace,
                    members,
                });
            }
            let (key_start, key_end) = self.scan_string()?;
            let key: String = serde_json::from_str(&self.src[key_start..key_end]).ok()?;
            self.skip_ws_and_comments()?;
            if self.pos >= self.end || self.bytes[self.pos] != b':' {
                return None;
            }
            self.pos += 1;
            let (val_start, val_end) = self.scan_value()?;
            self.skip_ws_and_comments()?;
            let comma_pos = if self.pos < self.end && self.bytes[self.pos] == b',' {
                let c = self.pos;
                self.pos += 1;
                Some(c)
            } else {
                None
            };
            members.push(MemberSpan {
                key,
                key_start,
                val_start,
                val_end,
                comma_pos,
            });
            if comma_pos.is_none() {
                self.skip_ws_and_comments()?;
                if self.pos < self.end && self.bytes[self.pos] == b'}' {
                    let close_brace = self.pos;
                    self.pos += 1;
                    return Some(ObjSpan {
                        open_brace,
                        close_brace,
                        members,
                    });
                }
                return None;
            }
        }
    }

    fn parse_root_object(&mut self) -> Option<ObjSpan> {
        let obj = self.parse_object()?;
        self.skip_ws_and_comments()?;
        if self.pos == self.end {
            Some(obj)
        } else {
            None
        }
    }
}

fn detect_newline(src: &str) -> &'static str {
    if src.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn detect_indent_unit(src: &str) -> String {
    let mut min_spaces: Option<usize> = None;
    for line in src.lines() {
        if line.starts_with('\t') {
            return "\t".to_string();
        }
        let spaces = line.bytes().take_while(|b| *b == b' ').count();
        if spaces > 0 && spaces < line.len() {
            let trimmed = &line[spaces..];
            if !trimmed.starts_with('*') {
                min_spaces = Some(match min_spaces {
                    Some(cur) => cur.min(spaces),
                    None => spaces,
                });
            }
        }
    }
    " ".repeat(min_spaces.unwrap_or(2).clamp(1, 8))
}

fn line_start_of(src: &str, pos: usize) -> usize {
    src[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

fn is_first_on_line(src: &str, pos: usize) -> bool {
    let start = line_start_of(src, pos);
    src[start..pos]
        .bytes()
        .all(|b| b == b' ' || b == b'\t')
}

fn line_indent_at(src: &str, pos: usize) -> String {
    let start = line_start_of(src, pos);
    src[start..pos]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

fn format_json_with_indent(
    value: &Value,
    base_indent: &str,
    indent_unit: &str,
    nl: &str,
) -> String {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            let child_indent = format!("{base_indent}{indent_unit}");
            let mut parts = Vec::with_capacity(map.len());
            for (k, v) in map {
                let key_str = serde_json::to_string(k).unwrap_or_else(|_| "\"\"".into());
                let val_str = format_json_with_indent(v, &child_indent, indent_unit, nl);
                parts.push(format!("{child_indent}{key_str}: {val_str}"));
            }
            format!("{{{nl}{}{nl}{base_indent}}}", parts.join(&format!(",{nl}")))
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                return "[]".to_string();
            }
            // 短标量数组保持单行（与常见手写 args: ["-u", "s.py"] 习惯一致）
            if arr.len() <= 6
                && arr
                    .iter()
                    .all(|item| !item.is_object() && !item.is_array())
            {
                let inline: Vec<String> = arr
                    .iter()
                    .map(|item| serde_json::to_string(item).unwrap_or_else(|_| "null".into()))
                    .collect();
                let joined = format!("[{}]", inline.join(", "));
                if joined.len() <= 80 {
                    return joined;
                }
            }
            let child_indent = format!("{base_indent}{indent_unit}");
            let parts: Vec<String> = arr
                .iter()
                .map(|item| {
                    format!(
                        "{child_indent}{}",
                        format_json_with_indent(item, &child_indent, indent_unit, nl)
                    )
                })
                .collect();
            format!("[{nl}{}{nl}{base_indent}]", parts.join(&format!(",{nl}")))
        }
        _ => serde_json::to_string(value).unwrap_or_else(|_| "null".into()),
    }
}

/// 在同一行内跳过水平空白与行尾注释（`//...` 或单行 `/*...*/`），返回行尾换行符之前的字节偏移。
fn end_of_same_line_ws_and_comment(src: &str, start: usize, limit: usize) -> usize {
    let bytes = src.as_bytes();
    let mut i = start;
    while i < limit {
        let b = bytes[i];
        if b == b' ' || b == b'\t' {
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < limit && bytes[i + 1] == b'/' {
            i += 2;
            while i < limit && bytes[i] != b'\r' && bytes[i] != b'\n' {
                i += 1;
            }
            break;
        }
        if b == b'/' && i + 1 < limit && bytes[i + 1] == b'*' {
            let comment_start = i;
            i += 2;
            while i + 1 < limit && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(limit);
            if src[comment_start..i].contains('\n') {
                break;
            }
            continue;
        }
        break;
    }
    i
}

fn insert_member_into_obj(
    src: &str,
    obj: &ObjSpan,
    depth: usize,
    key: &str,
    value: &Value,
    indent_unit: &str,
    nl: &str,
) -> String {
    let key_json = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".into());
    let (close_indent, member_indent) = if let Some(first) = obj.members.first() {
        if is_first_on_line(src, first.key_start) {
            let m_ind = line_indent_at(src, first.key_start);
            let c_ind = if is_first_on_line(src, obj.close_brace) {
                line_indent_at(src, obj.close_brace)
            } else {
                indent_unit.repeat(depth)
            };
            (c_ind, m_ind)
        } else {
            (indent_unit.repeat(depth), indent_unit.repeat(depth + 1))
        }
    } else {
        let c_ind = if is_first_on_line(src, obj.close_brace) {
            line_indent_at(src, obj.close_brace)
        } else {
            indent_unit.repeat(depth)
        };
        let m_ind = format!("{c_ind}{indent_unit}");
        (c_ind, m_ind)
    };

    let val_str = format_json_with_indent(value, &member_indent, indent_unit, nl);
    let member_text = format!("{member_indent}{key_json}: {val_str}");

    if obj.members.is_empty() {
        if !has_comment_in_span(src, obj.open_brace + 1, obj.close_brace) {
            return format!(
                "{}{nl}{member_text}{nl}{close_indent}{}",
                &src[..obj.open_brace + 1],
                &src[obj.close_brace..]
            );
        }
        // 空对象内含注释：保留全部注释，在 `}` 之前插入新条目
        let mut ws_start = obj.close_brace;
        while ws_start > obj.open_brace + 1
            && matches!(src.as_bytes()[ws_start - 1], b' ' | b'\t')
        {
            ws_start -= 1;
        }
        let has_leading_nl = ws_start > obj.open_brace + 1 && src.as_bytes()[ws_start - 1] == b'\n';
        if has_leading_nl {
            return format!(
                "{}{member_text}{nl}{close_indent}{}",
                &src[..ws_start],
                &src[obj.close_brace..]
            );
        }
        return format!(
            "{}{nl}{member_text}{nl}{close_indent}{}",
            &src[..obj.close_brace],
            &src[obj.close_brace..]
        );
    }

    let last = obj.members.last().unwrap();
    if let Some(cpos) = last.comma_pos {
        let after_line = end_of_same_line_ws_and_comment(src, cpos + 1, obj.close_brace);
        if !src[cpos + 1..obj.close_brace].contains('\n') {
            format!(
                "{}{nl}{member_text}{nl}{close_indent}{}",
                &src[..after_line],
                &src[obj.close_brace..]
            )
        } else {
            format!(
                "{}{nl}{member_text}{}",
                &src[..after_line],
                &src[after_line..]
            )
        }
    } else {
        let after_line = end_of_same_line_ws_and_comment(src, last.val_end, obj.close_brace);
        if !src[last.val_end..obj.close_brace].contains('\n') {
            format!(
                "{},{}{nl}{member_text}{nl}{close_indent}{}",
                &src[..last.val_end],
                &src[last.val_end..after_line],
                &src[obj.close_brace..]
            )
        } else {
            format!(
                "{},{}{nl}{member_text}{}",
                &src[..last.val_end],
                &src[last.val_end..after_line],
                &src[after_line..]
            )
        }
    }
}

fn patch_jsonc_upsert(
    src: &str,
    root: &[String],
    name: &str,
    value: &Value,
) -> Option<(String, bool)> {
    let nl = detect_newline(src);
    let indent_unit = detect_indent_unit(src);
    let mut cur_obj = JsoncScanner::new(src).parse_root_object()?;
    let mut depth = 0usize;

    for (idx, seg) in root.iter().enumerate() {
        if let Some(m) = cur_obj.members.iter().find(|m| m.key == *seg) {
            let child = JsoncScanner::new_slice(src, m.val_start, m.val_end).parse_root_object()?;
            cur_obj = child;
            depth = idx + 1;
        } else {
            let mut inner = JsonMap::new();
            inner.insert(name.to_string(), value.clone());
            let mut wrapped = Value::Object(inner);
            for rem_seg in root[idx + 1..].iter().rev() {
                let mut parent = JsonMap::new();
                parent.insert(rem_seg.clone(), wrapped);
                wrapped = Value::Object(parent);
            }
            let patched =
                insert_member_into_obj(src, &cur_obj, depth, seg, &wrapped, &indent_unit, nl);
            return Some((patched, false));
        }
    }

    if let Some(m) = cur_obj.members.iter().find(|m| m.key == name) {
        let stripped_interior = has_comment_in_span(src, m.val_start, m.val_end);
        let base_indent = if is_first_on_line(src, m.key_start) {
            line_indent_at(src, m.key_start)
        } else {
            indent_unit.repeat(depth + 1)
        };
        let val_str = format_json_with_indent(value, &base_indent, &indent_unit, nl);
        let patched = format!("{}{}{}", &src[..m.val_start], val_str, &src[m.val_end..]);
        Some((patched, stripped_interior))
    } else {
        let patched =
            insert_member_into_obj(src, &cur_obj, depth, name, value, &indent_unit, nl);
        Some((patched, false))
    }
}

fn patch_jsonc_remove(src: &str, root: &[String], name: &str) -> Option<(String, bool)> {
    let mut cur_obj = JsoncScanner::new(src).parse_root_object()?;
    for seg in root {
        let m = cur_obj.members.iter().find(|m| m.key == *seg)?;
        cur_obj = JsoncScanner::new_slice(src, m.val_start, m.val_end).parse_root_object()?;
    }
    let idx = cur_obj.members.iter().position(|m| m.key == name)?;
    let target = &cur_obj.members[idx];
    let stripped = has_comment_in_span(src, target.key_start, target.val_end);

    let start = if is_first_on_line(src, target.key_start) {
        line_start_of(src, target.key_start)
    } else {
        target.key_start
    };
    let mut end = target.comma_pos.map(|c| c + 1).unwrap_or(target.val_end);
    if start < target.key_start {
        // 如果整行只有该条目，顺带清理尾部水平空白与一个换行符
        let bytes = src.as_bytes();
        let mut p = end;
        while p < cur_obj.close_brace && (bytes[p] == b' ' || bytes[p] == b'\t') {
            p += 1;
        }
        if p < cur_obj.close_brace && bytes[p] == b'\r' {
            p += 1;
        }
        if p < cur_obj.close_brace && bytes[p] == b'\n' {
            end = p + 1;
        }
    }

    if idx > 0 && idx + 1 == cur_obj.members.len() && target.comma_pos.is_none() {
        if let Some(prev_comma) = cur_obj.members[idx - 1].comma_pos {
            let patched = format!(
                "{}{}{}",
                &src[..prev_comma],
                &src[prev_comma + 1..start],
                &src[end..]
            );
            return Some((patched, stripped));
        }
    }

    let patched = format!("{}{}", &src[..start], &src[end..]);
    Some((patched, stripped))
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
    fn jsonc_surgical_patch_preserves_comments_and_four_space_indent() {
        let src = "{\n    // 顶层注释保留\n    \"mcpServers\": {\n        /* 兄弟注释保留 */\n        \"keep\": {\n            \"command\": \"npx\"\n        } // 行尾注释保留\n    }\n}\n";
        let mut doc = ConfigDoc::parse(Format::Jsonc, src).unwrap();
        let r = root("mcpServers");
        doc.upsert(&r, "added", &json!({"command": "python"}), MergeMode::Merge)
            .unwrap();
        assert!(!doc.lost_comments(), "外部与兄弟注释均未丢失");
        let out = doc.render();
        assert!(out.contains("// 顶层注释保留"));
        assert!(out.contains("/* 兄弟注释保留 */"));
        assert!(out.contains("// 行尾注释保留"));
        assert!(out.contains("        \"added\": {\n            \"command\": \"python\"\n        }"));
    }

    #[test]
    fn jsonc_flags_lost_comments_only_when_interior_comment_overwritten() {
        let src = "{\n  // 外部注释\n  \"mcpServers\": {\n    \"alpha\": {\n      /* 内部注释 */\n      \"command\": \"old\"\n    }\n  }\n}\n";
        let mut doc = ConfigDoc::parse(Format::Jsonc, src).unwrap();
        let r = root("mcpServers");
        doc.upsert(&r, "alpha", &json!({"command": "new"}), MergeMode::Merge)
            .unwrap();
        assert!(doc.lost_comments(), "被替换条目内部含注释时必须标记丢失");
        let out = doc.render();
        assert!(out.contains("// 外部注释"), "外部注释仍应完整保留");
    }

    #[test]
    fn three_way_merge_deletes_removed_managed_keys_while_preserving_user_fields() {
        let existing = json!({
            "command": "python",
            "args": ["old.py"],
            "env": {
                "MANAGED_KEEP": "1",
                "MANAGED_REMOVED": "secret",
                "USER_CUSTOM": "mine"
            },
            "alwaysAllow": ["read_file"]
        });
        let prev_managed = ManagedKeys::from_value(&json!({
            "command": "python",
            "args": ["old.py"],
            "env": {
                "MANAGED_KEEP": "1",
                "MANAGED_REMOVED": "secret"
            }
        }));
        let incoming = json!({
            "command": "python",
            "env": {
                "MANAGED_KEEP": "2"
            }
        });
        let merged = merge_json_3way(&existing, &incoming, Some(&prev_managed));
        assert!(merged.get("args").is_none(), "已移除的托管键 args 应被清理");
        assert_eq!(merged["env"]["MANAGED_KEEP"], "2");
        assert!(
            merged["env"].get("MANAGED_REMOVED").is_none(),
            "已移除的托管环境变量应被清理"
        );
        assert_eq!(
            merged["env"]["USER_CUSTOM"], "mine",
            "用户手写环境变量必须保留"
        );
        assert_eq!(
            merged["alwaysAllow"],
            json!(["read_file"]),
            "用户手写扩展字段必须保留"
        );
    }

    #[test]
    fn yaml_format_round_trips() {
        let mut doc = ConfigDoc::parse(
            Format::Yaml,
            "model: gpt-4\nmcpServers:\n  keep:\n    command: npx\n",
        )
        .unwrap();
        let r = root("mcpServers");
        doc.upsert(&r, "added", &json!({"command": "uvx"}), MergeMode::Merge)
            .unwrap();
        let out = doc.render();
        let reparsed = ConfigDoc::parse(Format::Yaml, &out).unwrap();
        assert_eq!(reparsed.get(&r, "keep").unwrap()["command"], "npx");
        assert_eq!(reparsed.get(&r, "added").unwrap()["command"], "uvx");
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
