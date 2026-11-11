//! Agent → 配置库：原文留在后端，预览绑定两端快照；凭据字段可映射为环境变量，或按用户选择原样复制。
use crate::{App, AppError};
use mcp_core::{
    adapters::{self, Adapter, EnabledKey, EntryStyle},
    catalog,
    document::ConfigDoc,
    model::{Auth, Catalog, McpEntry, Platform, Scope, SecretSource},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const TTL: Duration = Duration::from_secs(600);
const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// 与 `mcp_core::redact` 的隐藏标记一致；原样复制的凭据在预览中只显示该标记。
const HIDDEN: &str = "[已隐藏]";
fn fail(message: &str) -> AppError {
    AppError::Other(message.into())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportField {
    pub id: String,
    pub label: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItem {
    pub name: String,
    pub transport: Option<String>,
    pub conflict: bool,
    pub blocked: Option<String>,
    pub fields: Vec<ImportField>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportScan {
    pub scan_id: String,
    pub source_path: String,
    pub catalog_path: String,
    pub catalog_exists: bool,
    pub items: Vec<ImportItem>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportChoice {
    pub source_name: String,
    pub action: ImportAction,
    pub target_name: String,
    /// 凭据字段 → 环境变量名。
    pub bindings: BTreeMap<String, String>,
    /// 原样复制原值的凭据字段；每个凭据字段必须且只能出现在 `bindings` 或此处之一。
    #[serde(default)]
    pub copy_fields: BTreeSet<String>,
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImportAction {
    Skip,
    Add,
    Rename,
    Replace,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportChange {
    pub source_name: String,
    pub target_name: String,
    pub action: ImportAction,
    pub before: Option<Value>,
    pub after: Value,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub preview_id: String,
    pub catalog_path: String,
    pub changes: Vec<ImportChange>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: usize,
    pub warnings: Vec<String>,
}

// 不实现 Serialize / Debug：原始文件快照不允许进入 IPC 或诊断输出。
struct Snapshot {
    client: String,
    scope: Scope,
    source: PathBuf,
    source_key: String,
    source_text: String,
    target: PathBuf,
    target_key: String,
    target_text: Option<String>,
    catalog: Catalog,
    settings: Option<String>,
    custom_adapters: Option<String>,
    context: (PathBuf, PathBuf, Option<PathBuf>, Platform, PathBuf),
    items: Vec<(ImportItem, Option<Converted>)>,
    created: Instant,
}
/// 同样不实现 Serialize / Debug：`originals` 保存凭据原值，只在后端用于“原样复制”。
struct Converted {
    /// 凭据字段已清空、可安全展示的条目。
    safe: Value,
    /// 凭据字段 id → (原值, 预览中显示的隐藏值)。
    originals: BTreeMap<String, (Value, Value)>,
}
struct Prepared {
    scan_id: String,
    catalog: Catalog,
    imported: usize,
    created: Instant,
}
#[derive(Default)]
pub struct ImportSessions {
    serial: u64,
    scans: BTreeMap<String, Snapshot>,
    previews: BTreeMap<String, Prepared>,
}

impl ImportSessions {
    fn id(&mut self) -> String {
        self.serial += 1;
        format!("{}-{}", std::process::id(), self.serial)
    }
    fn prune(&mut self) {
        self.scans.retain(|_, s| s.created.elapsed() < TTL);
        self.previews
            .retain(|_, p| p.created.elapsed() < TTL && self.scans.contains_key(&p.scan_id));
    }
    pub fn cancel(&mut self, id: &str) {
        self.scans.remove(id);
        self.previews.retain(|_, p| p.scan_id != id);
    }
    pub fn scan(
        &mut self,
        app: &App,
        target: &Path,
        client: &str,
        scope: Scope,
    ) -> Result<ImportScan, AppError> {
        self.prune();
        if self.scans.len() >= 8 {
            return Err(fail("导入草稿过多，请取消旧草稿或稍后重试"));
        }
        if !target.is_absolute() {
            return Err(fail("导入配置库必须使用绝对路径"));
        }
        let adapter_set = app.adapter_set()?;
        let adapter = adapter_set.by_id(client).ok_or_else(|| fail("未知 Agent"))?;
        if let Some(reason) = app.check_scope_block(adapter, scope) {
            return Err(fail(&reason));
        }
        let loc = app
            .target_location(adapter, scope)
            .ok_or_else(|| fail("无法定位来源配置，请先选择项目目录"))?;
        let source = loc.path;
        let source_key = mcp_io::file_key(&source)?;
        let target_key = mcp_io::file_key(target)?;
        // 即使另一 Agent 的文件为空或尚不存在，也不能将其当作配置库覆盖。
        for other_client in adapter_set.all() {
            // local 的 #projects.<proj> 是虚拟位置，其实际 .claude.json 已在 user 范围保护。
            for cand_scope in [Scope::User, Scope::Project] {
                let candidates = other_client.path_candidates(cand_scope, app.platform);
                for path in app.env.expand_all(&candidates, app.platform) {
                    if mcp_io::file_key(&path)? == target_key {
                        return Err(fail("配置库不能使用任何已知 Agent 的配置路径"));
                    }
                }
            }
        }
        if source_key == target_key
            || target_key == mcp_io::file_key(&app.settings_path())?
            || target_key == mcp_io::file_key(&app.custom_adapters_path())?
            || target_key == mcp_io::file_key(&app.engine.managed_state_path())?
            || target_key == mcp_io::file_key(&app.engine.audit_path())?
        {
            return Err(fail("配置库不能覆盖来源 Agent 文件、软件设置或审计日志"));
        }
        let source_text =
            read(&source)?.ok_or_else(|| fail("源配置文件不存在，请重新读取 Agent"))?;
        let doc = ConfigDoc::parse(
            adapter.format_for_path(&source),
            source_text.trim_start_matches('\u{feff}'),
        )
        .map_err(|_| fail("源配置解析失败，未显示原文"))?;
        let entries = doc
            .entries(&loc.root_key)
            .map_err(|_| fail("来源 MCP 容器结构无效"))?;
        let target_text = read(target)?;
        let catalog = match &target_text {
            Some(text) => {
                catalog::load(text)
                    .map_err(|_| fail("目标配置库损坏或结构无效，未覆盖原文"))?
                    .0
            }
            None => Catalog::default(),
        };
        let mut items = Vec::new();
        for (name, raw) in entries {
            let conflict = catalog.entries.iter().any(|entry| entry.name == name);
            let (item, safe) = match convert(adapter, &name, &raw, scope) {
                Ok((safe, fields, originals)) => (
                    ImportItem {
                        name,
                        conflict,
                        transport: safe["transport"].as_str().map(str::to_string),
                        blocked: None,
                        fields,
                    },
                    Some(Converted { safe, originals }),
                ),
                Err(reason) => (
                    ImportItem {
                        name,
                        conflict,
                        transport: None,
                        blocked: Some(reason.into()),
                        fields: Vec::new(),
                    },
                    None,
                ),
            };
            items.push((item, safe));
        }
        items.sort_by(|a, b| a.0.name.cmp(&b.0.name));
        let scan_id = self.id();
        let result = ImportScan {
            scan_id: scan_id.clone(),
            source_path: source.display().to_string(),
            catalog_path: target.display().to_string(),
            catalog_exists: target_text.is_some(),
            items: items.iter().map(|(item, _)| item.clone()).collect(),
        };
        self.scans.insert(
            scan_id,
            Snapshot {
                client: client.into(),
                scope,
                source,
                source_key,
                source_text,
                target: target.to_path_buf(),
                target_key,
                target_text,
                catalog,
                settings: read(&app.settings_path())?,
                custom_adapters: read(&app.custom_adapters_path())?,
                context: context(app),
                items,
                created: Instant::now(),
            },
        );
        Ok(result)
    }

    pub fn preview(
        &mut self,
        app: &App,
        scan_id: &str,
        choices: &[ImportChoice],
    ) -> Result<ImportPreview, AppError> {
        self.prune();
        // 即使本次校验失败，也不能保留先前可提交的预览。
        self.previews.retain(|_, p| p.scan_id != scan_id);
        let snapshot = self
            .scans
            .get(scan_id)
            .ok_or_else(|| fail("导入草稿已失效，请重新扫描"))?;
        verify(app, snapshot)?;
        let mut seen_source = BTreeSet::new();
        let mut seen_target = BTreeSet::new();
        let mut catalog = snapshot.catalog.clone();
        let mut changes = Vec::new();
        for choice in choices {
            if !seen_source.insert(&choice.source_name) {
                return Err(fail("同一来源不能重复选择"));
            }
            let (item, safe) = snapshot
                .items
                .iter()
                .find(|(i, _)| i.name == choice.source_name)
                .ok_or_else(|| fail("选择包含未知来源条目"))?;
            if choice.action == ImportAction::Skip {
                continue;
            }
            let converted = safe
                .as_ref()
                .ok_or_else(|| fail("所选条目包含无法安全转换的字段"))?;
            let name = choice.target_name.trim();
            if name.is_empty()
                || name != choice.target_name
                || name.contains(char::is_control)
                || name == "[已隐藏]"
            {
                return Err(fail("目标名称无效"));
            }
            if !seen_target.insert(name.to_string()) {
                return Err(fail("本次导入的目标名称重复"));
            }
            let existing = snapshot.catalog.entries.iter().find(|e| e.name == name);
            match choice.action {
                ImportAction::Add if existing.is_some() || name != item.name => {
                    return Err(fail("新增名称已存在或发生变化，请明确选择重命名或替换"))
                }
                ImportAction::Rename if existing.is_some() || name == item.name => {
                    return Err(fail("重命名必须使用新的唯一名称"))
                }
                ImportAction::Replace if existing.is_none() || name != item.name => {
                    return Err(fail("替换仅适用于明确选中的同名条目"))
                }
                _ => {}
            }
            let known: BTreeSet<&str> = item.fields.iter().map(|f| f.id.as_str()).collect();
            if choice.bindings.len() + choice.copy_fields.len() != item.fields.len()
                || choice
                    .bindings
                    .keys()
                    .any(|id| !known.contains(id.as_str()))
                || choice
                    .copy_fields
                    .iter()
                    .any(|id| !known.contains(id.as_str()))
                || choice
                    .bindings
                    .keys()
                    .any(|id| choice.copy_fields.contains(id))
            {
                return Err(fail("请为每个凭据字段选择原样复制或环境变量映射"));
            }
            // after 写入配置库；shown 返回前端预览，原样复制的凭据只显示隐藏标记。
            let mut after = converted.safe.clone();
            let mut shown = converted.safe.clone();
            for field in &item.fields {
                if choice.copy_fields.contains(&field.id) {
                    let (original, hidden) = converted
                        .originals
                        .get(&field.id)
                        .ok_or_else(|| fail("导入字段映射失效，请重新扫描"))?;
                    if field.id == "/auth/token_env" {
                        after["auth"] = serde_json::json!({"type": "bearer", "token": original});
                        shown["auth"] = serde_json::json!({"type": "bearer", "token": hidden});
                    } else {
                        for (target, value) in [(&mut after, original), (&mut shown, hidden)] {
                            *target
                                .pointer_mut(&field.id)
                                .ok_or_else(|| fail("导入字段映射失效，请重新扫描"))? =
                                value.clone();
                        }
                    }
                    continue;
                }
                let env = choice
                    .bindings
                    .get(&field.id)
                    .filter(|name| env_name(name))
                    .ok_or_else(|| fail("环境变量名仅允许字母、数字和下划线，且不能以数字开头"))?;
                let slot = after
                    .pointer_mut(&field.id)
                    .ok_or_else(|| fail("导入字段映射失效，请重新扫描"))?;
                let value = if field.id == "/auth/token_env" {
                    env.to_string()
                } else if field.id.starts_with("/args/")
                    && slot
                        .as_str()
                        .is_some_and(|s| s.starts_with('-') && s.ends_with('='))
                {
                    format!("{}${{ENV:{env}}}", slot.as_str().unwrap())
                } else {
                    format!("${{ENV:{env}}}")
                };
                *slot = Value::String(value.clone());
                *shown
                    .pointer_mut(&field.id)
                    .ok_or_else(|| fail("导入字段映射失效，请重新扫描"))? = Value::String(value);
            }
            after["name"] = Value::String(name.into());
            shown["name"] = Value::String(name.into());
            let entry: McpEntry =
                serde_json::from_value(after.clone()).map_err(|_| fail("导入结果结构无效"))?;
            let before = existing.map(|e| {
                mcp_core::redact::config_entry(&serde_json::to_value(e).unwrap_or(Value::Null))
            });
            catalog.entries.retain(|e| e.name != name);
            catalog.entries.push(entry);
            changes.push(ImportChange {
                source_name: item.name.clone(),
                target_name: name.into(),
                action: choice.action,
                before,
                after: shown,
            });
        }
        if changes.is_empty() {
            return Err(fail("尚未选择可导入的条目，不会写入配置库"));
        }
        if catalog::validate(&catalog)
            .iter()
            .any(|issue| issue.is_error())
        {
            return Err(fail("导入后的配置库校验失败"));
        }
        if self.previews.len() >= 8 {
            return Err(fail("预览过多，请取消旧导入草稿"));
        }
        let imported = changes.len();
        let path = snapshot.target.display().to_string();
        let id = self.id();
        self.previews.insert(
            id.clone(),
            Prepared {
                scan_id: scan_id.into(),
                catalog,
                imported,
                created: Instant::now(),
            },
        );
        Ok(ImportPreview {
            preview_id: id,
            catalog_path: path,
            changes,
        })
    }

    pub fn commit(&mut self, app: &App, id: &str) -> Result<ImportResult, AppError> {
        self.prune();
        let prepared = self
            .previews
            .remove(id)
            .ok_or_else(|| fail("导入预览已失效或已提交，请重新预览"))?;
        let snapshot = self
            .scans
            .remove(&prepared.scan_id)
            .ok_or_else(|| fail("导入草稿已失效"))?;
        self.previews.retain(|_, p| p.scan_id != prepared.scan_id);
        verify(app, &snapshot)?;
        let warnings = app.save_catalog_checked(
            &snapshot.target,
            &prepared.catalog,
            Some(snapshot.target_text.as_deref()),
        )?;
        Ok(ImportResult {
            imported: prepared.imported,
            warnings,
        })
    }
}

fn context(app: &App) -> (PathBuf, PathBuf, Option<PathBuf>, Platform, PathBuf) {
    (
        app.engine.home().to_path_buf(),
        app.env.home.clone(),
        app.env.project_root.clone(),
        app.platform,
        app.engine.backups_dir(),
    )
}
fn verify(app: &App, snapshot: &Snapshot) -> Result<(), AppError> {
    if snapshot.created.elapsed() >= TTL
        || context(app) != snapshot.context
        || read(&app.settings_path())? != snapshot.settings
        || read(&app.custom_adapters_path())? != snapshot.custom_adapters
    {
        return Err(fail("导入上下文或软件设置已变化，请重新扫描"));
    }
    let adapter_set = app.adapter_set()?;
    let adapter = adapter_set
        .by_id(&snapshot.client)
        .ok_or_else(|| fail("未知来源"))?;
    let path = app
        .target_path(adapter, snapshot.scope)
        .ok_or_else(|| fail("来源路径已变化"))?;
    if mcp_io::file_key(&path)? != snapshot.source_key
        || mcp_io::file_key(&snapshot.source)? != snapshot.source_key
        || read(&snapshot.source)?.as_deref() != Some(snapshot.source_text.as_str())
    {
        return Err(fail("源配置已变化，请重新扫描并预览"));
    }
    if mcp_io::file_key(&snapshot.target)? != snapshot.target_key
        || read(&snapshot.target)? != snapshot.target_text
    {
        return Err(fail("目标配置库已变化，请重新扫描并预览"));
    }
    Ok(())
}
fn read(path: &Path) -> Result<Option<String>, AppError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(fail("无法读取导入文件，请检查路径和权限")),
    };
    let mut text = String::new();
    file.take(MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| fail("导入文件编码无效或无法读取"))?;
    if text.len() as u64 > MAX_BYTES {
        return Err(fail("导入文件超过 4 MiB，请先精简文件"));
    }
    Ok(Some(text))
}
fn env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
fn reference(value: &str) -> Option<String> {
    if ["${HOME}", "${HOME_DIR}", "${PROJECT_ROOT}", "${CWD}"].contains(&value) {
        return Some(value.into());
    }
    let name = value
        .strip_prefix("${ENV:")
        .and_then(|v| v.strip_suffix('}'))
        .or_else(|| {
            value
                .strip_prefix("${env:")
                .and_then(|v| v.strip_suffix('}'))
        })
        .or_else(|| {
            value
                .strip_prefix("{env:")
                .and_then(|v| v.strip_suffix('}'))
        })
        .or_else(|| value.strip_prefix("${").and_then(|v| v.strip_suffix('}')))?;
    env_name(name).then(|| format!("${{ENV:{name}}}"))
}
fn normalize_references(value: &str) -> String {
    let mut rest = value;
    let mut out = String::with_capacity(value.len());
    while !rest.is_empty() {
        if rest.starts_with("${") || rest.starts_with("{env:") {
            if let Some(end) = rest.find('}') {
                let token = &rest[..=end];
                out.push_str(reference(token).as_deref().unwrap_or(token));
                rest = &rest[end + 1..];
                continue;
            }
            out.push_str(rest);
            break;
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

fn field(
    value: &mut Value,
    shown: &Value,
    id: String,
    label: String,
    inline_prefix: Option<&str>,
    fields: &mut Vec<ImportField>,
    originals: &mut BTreeMap<String, (Value, Value)>,
) {
    if let Some(normalized) = value.as_str().and_then(reference) {
        *value = Value::String(normalized);
    } else if *value != *shown {
        originals.insert(id.clone(), (value.clone(), shown.clone()));
        *value = Value::String(inline_prefix.unwrap_or_default().into());
        fields.push(ImportField { id, label });
    } else if let Some(text) = value.as_str() {
        *value = Value::String(normalize_references(text));
    }
}
fn pointer_key(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

type ConvertedParts = (Value, Vec<ImportField>, BTreeMap<String, (Value, Value)>);

fn convert(
    adapter: &dyn Adapter,
    name: &str,
    raw: &Value,
    scope: Scope,
) -> Result<ConvertedParts, &'static str> {
    strict(adapter, raw)?;
    let mut entry = adapter
        .decode_entry(name, raw)
        .map_err(|_| "无法转换来源条目")?;
    entry.scope = Some(scope);
    let auth_token = match &entry.auth {
        Some(Auth::Bearer {
            token: SecretSource::Literal(token),
        }) => Some(token.clone()),
        Some(Auth::Bearer {
            token: SecretSource::EnvRef(name),
        }) if !env_name(name) => return Err("来源认证环境变量名无效"),
        _ => None,
    };
    if auth_token.is_some() {
        entry.auth = Some(Auth::env_ref("IMPORT_PENDING"));
    }
    let mut safe = serde_json::to_value(entry).map_err(|_| "无法转换来源条目")?;
    let mut fields = Vec::new();
    let mut originals = BTreeMap::new();
    if let Some(token) = auth_token {
        safe["auth"]["token_env"] = Value::String(String::new());
        originals.insert(
            "/auth/token_env".to_string(),
            (Value::String(token), Value::String(HIDDEN.into())),
        );
        fields.push(ImportField {
            id: "/auth/token_env".into(),
            label: "Bearer 凭据".into(),
        });
    }
    let shown = mcp_core::redact::config_entry(&safe);
    for key in ["command", "cwd", "url"] {
        if let Some(value) = safe.get_mut(key) {
            field(
                value,
                &shown[key],
                format!("/{key}"),
                key.into(),
                None,
                &mut fields,
                &mut originals,
            );
        }
    }
    if let Some(args) = safe.get_mut("args").and_then(Value::as_array_mut) {
        let prefixes = mcp_core::redact::inline_option_prefixes(args);
        for (i, value) in args.iter_mut().enumerate() {
            field(
                value,
                &shown["args"][i],
                format!("/args/{i}"),
                format!("参数 {}", i + 1),
                prefixes[i].as_deref(),
                &mut fields,
                &mut originals,
            );
        }
    }
    for key in ["env", "headers"] {
        if let Some(map) = safe.get_mut(key).and_then(Value::as_object_mut) {
            for (name, value) in map {
                field(
                    value,
                    &shown[key][name.as_str()],
                    format!("/{key}/{}", pointer_key(name)),
                    format!("{key}.{name}"),
                    None,
                    &mut fields,
                    &mut originals,
                );
            }
        }
    }
    Ok((safe, fields, originals))
}

/// 通用解码器容许读旧配置；导入必须额外拒绝丢字段、类型过滤和有歧义的别名。
fn strict(adapter: &dyn Adapter, raw: &Value) -> Result<(), &'static str> {
    let obj = raw.as_object().ok_or("来源条目不是对象")?;
    let spec = adapter.spec();
    let mut allowed = vec![
        "command",
        "args",
        "cwd",
        "url",
        "serverUrl",
        "env",
        "environment",
        "headers",
        "type",
        "bearer_token_env_var",
    ];
    allowed.extend_from_slice(adapters::KNOWN_CLIENT_EXTENSION_KEYS);
    if let Some(key) = spec.timeout_field.as_deref() {
        allowed.push(key);
    }
    match spec.enabled_key {
        EnabledKey::Enabled => allowed.push("enabled"),
        EnabledKey::Disabled => allowed.push("disabled"),
        _ => {}
    }
    if obj.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err("包含无法无损转换的字段，请手动整理后重试");
    }
    for list_key in ["alwaysAllow", "autoApprove", "disabledTools"] {
        if let Some(v) = obj.get(list_key) {
            let Some(arr) = v.as_array() else {
                return Err("扩展工具列表字段必须是字符串数组");
            };
            if !arr.iter().all(Value::is_string) {
                return Err("扩展工具列表元素必须全部为字符串");
            }
            let probe = serde_json::json!({ "args": v });
            if mcp_core::redact::config_entry(&probe)["args"] != *v {
                return Err("扩展工具列表包含疑似敏感凭据，拒绝自动导入");
            }
        }
    }
    if obj
        .get("tool_timeout_sec")
        .is_some_and(|v| v.as_u64().is_none())
    {
        return Err("工具超时扩展字段类型无效");
    }
    if ["url", "serverUrl"]
        .iter()
        .filter(|k| obj.contains_key(**k))
        .count()
        > 1
        || ["env", "environment"]
            .iter()
            .filter(|k| obj.contains_key(**k))
            .count()
            > 1
    {
        return Err("存在有歧义的字段别名");
    }
    let remote = obj.contains_key("url") || obj.contains_key("serverUrl");
    if remote
        && ["command", "args", "cwd"]
            .iter()
            .any(|key| obj.contains_key(*key))
    {
        return Err("远程与本地字段混合，无法安全转换");
    }
    if let Some(kind) = obj.get("type") {
        let supported = if remote {
            ["http", "sse", "remote"].contains(&kind.as_str().unwrap_or(""))
        } else {
            ["stdio", "local"].contains(&kind.as_str().unwrap_or(""))
        };
        if !supported {
            return Err("传输类型与来源字段不一致");
        }
    }
    for key in ["url", "serverUrl", "cwd"] {
        if let Some(v) = obj.get(key) {
            if !v.as_str().is_some_and(|s| !s.trim().is_empty()) {
                return Err("地址或工作目录类型无效");
            }
        }
    }
    if !remote {
        match obj.get("command") {
            Some(Value::String(s)) if !s.trim().is_empty() => {}
            Some(Value::Array(parts))
                if spec.entry_style == EntryStyle::CommandArray
                    && !parts.is_empty()
                    && parts.iter().all(Value::is_string)
                    && !parts[0].as_str().unwrap_or("").trim().is_empty()
                    && !obj.contains_key("args") => {}
            _ => return Err("命令结构无效或包含无法合并的参数"),
        }
        if obj
            .get("args")
            .is_some_and(|v| !v.as_array().is_some_and(|a| a.iter().all(Value::is_string)))
        {
            return Err("参数必须全部为字符串，不能过滤或丢弃");
        }
    }
    for key in ["env", "environment", "headers"] {
        if let Some(value) = obj.get(key) {
            let map = value.as_object().ok_or("环境变量或请求头必须是对象")?;
            if map.iter().any(|(name, value)| {
                !value.is_string()
                    || (key != "headers" && !env_name(name))
                    || (key == "headers"
                        && (name.is_empty()
                            || !name.chars().all(|c| {
                                c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)
                            })))
            }) {
                return Err("环境变量或请求头包含无效字段类型");
            }
        }
    }
    let auth_headers = obj
        .get("headers")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if auth_headers.len() > 1
        || (!auth_headers.is_empty() && obj.contains_key("bearer_token_env_var"))
    {
        return Err("存在多个认证来源，不能自动选择");
    }
    if let Some((_, value)) = auth_headers.first() {
        if !value
            .as_str()
            .and_then(|s| {
                s.strip_prefix("Bearer ")
                    .or_else(|| s.strip_prefix("bearer "))
            })
            .is_some_and(|s| !s.trim().is_empty())
        {
            return Err("仅支持可转换的 Bearer 认证，其他认证请手动整理");
        }
    }
    if obj
        .get("bearer_token_env_var")
        .is_some_and(|v| !v.as_str().is_some_and(env_name))
    {
        return Err("认证变量名无效");
    }
    if obj
        .get("enabled")
        .or_else(|| obj.get("disabled"))
        .is_some_and(|v| !v.is_boolean())
    {
        return Err("启用开关类型无效");
    }
    if spec
        .timeout_field
        .as_deref()
        .and_then(|key| obj.get(key))
        .is_some_and(|v| v.as_u64().is_none())
    {
        return Err("超时字段类型无效");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_drafts_and_previews_are_discarded() {
        let mut sessions = ImportSessions::default();
        sessions.previews.insert(
            "expired".into(),
            Prepared {
                scan_id: "missing".into(),
                catalog: Catalog::default(),
                imported: 1,
                created: Instant::now() - TTL - Duration::from_secs(1),
            },
        );
        sessions.prune();
        assert!(sessions.previews.is_empty());
    }
}
