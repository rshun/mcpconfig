//! 应用计划：把「清单条目 + 目标文件」编译成逐文件的变更方案。
//!
//! 纯函数：接收已有文本（可能是 `None`），返回**将要写入的内容**与差异，
//! 不碰文件系统。写盘由 `mcp-io` 负责。

use crate::adapters::{Adapter, Format};
use crate::diff::{self, DiffLine};
pub use crate::document::{ChangeKind, ManagedKeys, MergeMode};
use crate::document::{reconcile_entry, ConfigDoc};
use crate::error::Result;
use crate::model::{McpEntry, Scope};
use serde_json::Value;
use std::collections::BTreeMap;

/// 单条目的变更明细。
#[derive(Debug, Clone, PartialEq)]
pub struct EntryChange {
    pub name: String,
    pub kind: ChangeKind,
    /// 现有内容（新增时为 None）
    pub before: Option<Value>,
    /// 本次将写入的内容（合并后的最终值）
    pub after: Value,
    /// 本次由适配器编码生成的纯键骨架（不含用户未托管手写键，不含任何密钥值）
    pub managed_keys: ManagedKeys,
}

/// 一个目标文件的完整计划。
#[derive(Debug, Clone, PartialEq)]
pub struct FilePlan {
    pub client_id: String,
    pub client_name: String,
    pub scope: Scope,
    pub path: String,
    pub format: Format,
    /// 该文件内实际操作的容器根路径（例如 `["mcpServers"]` 或 `["projects", "<proj>", "mcpServers"]`）
    pub root_key: Vec<String>,
    /// 目标文件此前是否存在
    pub existed: bool,
    /// 非空表示本次不写该文件（含原因）
    pub blocked: Option<String>,
    pub changes: Vec<EntryChange>,
    pub old_content: String,
    pub new_content: String,
    pub line_diff: Vec<DiffLine>,
    pub warnings: Vec<String>,
}

/// 单文件规划高级选项（支持动态容器根键、三方合并键骨架基线与指定条目移除）。
#[derive(Debug, Clone, Default)]
pub struct PlanFileOptions<'a> {
    pub root_key_override: Option<&'a [String]>,
    pub managed_entries: Option<&'a BTreeMap<String, ManagedKeys>>,
    pub removals: &'a [String],
}

impl FilePlan {
    /// 是否有实际改动（决定是否需要备份与写盘）。
    pub fn has_write(&self) -> bool {
        self.blocked.is_none()
            && self.changes.iter().any(|c| c.kind != ChangeKind::Unchanged)
            && self.old_content != self.new_content
    }

    pub fn changed_count(&self) -> (usize, usize) {
        let new = self
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::New)
            .count();
        let upd = self
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Update)
            .count();
        (new, upd)
    }

    pub fn removed_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Removed)
            .count()
    }

    /// unified diff 文本（无改动时为空串）。
    pub fn diff_text(&self) -> String {
        let (old, new) = self.display_texts();
        diff::unified(&old, &new, 3)
    }

    /// 只展示本次目标条目，不带出源文件其他区域的凭据。
    pub fn display_texts(&self) -> (String, String) {
        let mut old = Vec::new();
        let mut new = Vec::new();
        for change in self
            .changes
            .iter()
            .filter(|c| c.kind != ChangeKind::Unchanged)
        {
            if change.kind == ChangeKind::Removed {
                if let Some(before) = &change.before {
                    let safe_before = crate::redact::config_entry(before);
                    old.push(serde_json::json!({ "name": change.name, "config": safe_before }));
                }
                continue;
            }
            let (before, after) = crate::redact::change(change.before.as_ref(), &change.after);
            if let Some(before) = before {
                old.push(serde_json::json!({ "name": change.name, "config": before }));
            }
            new.push(serde_json::json!({ "name": change.name, "config": after }));
        }
        let text = |entries: Vec<Value>| {
            if entries.is_empty() {
                String::new()
            } else {
                serde_json::to_string_pretty(&entries).unwrap_or_default()
            }
        };
        (text(old), text(new))
    }
}

/// 规划单个文件（默认选项）。
#[allow(clippy::too_many_arguments)]
pub fn plan_file(
    adapter: &dyn Adapter,
    path: impl Into<String>,
    scope: Scope,
    existing: Option<&str>,
    entries: &[McpEntry],
    mode: MergeMode,
    allow_comment_strip: bool,
) -> Result<FilePlan> {
    plan_file_with_options(
        adapter,
        path,
        scope,
        existing,
        entries,
        mode,
        allow_comment_strip,
        &PlanFileOptions::default(),
    )
}

/// 规划单个文件（支持自定义容器路径与三方合并托管键骨架）。
#[allow(clippy::too_many_arguments)]
pub fn plan_file_with_options(
    adapter: &dyn Adapter,
    path: impl Into<String>,
    scope: Scope,
    existing: Option<&str>,
    entries: &[McpEntry],
    mode: MergeMode,
    allow_comment_strip: bool,
    opts: &PlanFileOptions<'_>,
) -> Result<FilePlan> {
    let path = path.into();
    let format = adapter.format_for_path(std::path::Path::new(&path));
    let existed = existing.is_some();
    let old_content = existing.unwrap_or("").to_string();
    let root = match opts.root_key_override {
        Some(r) => r.to_vec(),
        None => adapter.root_key_for_path(scope, std::path::Path::new(&path)),
    };

    let mut plan = FilePlan {
        client_id: adapter.id().to_string(),
        client_name: adapter.display_name().to_string(),
        scope,
        path,
        format,
        root_key: root.clone(),
        existed,
        blocked: None,
        changes: Vec::new(),
        old_content: old_content.clone(),
        new_content: old_content.clone(),
        line_diff: Vec::new(),
        warnings: Vec::new(),
    };

    if !adapter.supports_scope(scope) {
        plan.blocked = Some(format!(
            "{} 不支持 {} 作用域",
            adapter.display_name(),
            scope.as_str()
        ));
        return Ok(plan);
    }

    let mut doc = match ConfigDoc::parse(format, old_content.trim_start_matches('\u{feff}')) {
        Ok(d) => d,
        Err(e) => {
            plan.blocked = Some(format!("现有文件解析失败，已跳过（不会重建）：{e}"));
            return Ok(plan);
        }
    };

    let mut unresolved_vars: Vec<String> = Vec::new();

    for entry in entries {
        let encoded = adapter.encode_entry_checked(entry)?;
        unresolved_vars.extend(encoded.unresolved_inline_secrets.iter().cloned());

        let prev_managed = opts.managed_entries.and_then(|m| m.get(&entry.name));
        let before = doc.get(&root, &entry.name);
        let (after, _) = reconcile_entry(before.as_ref(), &encoded.value, mode, prev_managed);
        let kind = doc.upsert_with_managed(&root, &entry.name, &encoded.value, mode, prev_managed)?;

        plan.changes.push(EntryChange {
            name: entry.name.clone(),
            kind,
            before,
            after,
            managed_keys: ManagedKeys::from_value(&encoded.value),
        });
    }

    for rem_name in opts.removals {
        if entries.iter().any(|e| &e.name == rem_name) {
            continue;
        }
        let before = doc.get(&root, rem_name);
        let removed = if before.is_some() {
            doc.remove(&root, rem_name)?
        } else {
            false
        };
        plan.changes.push(EntryChange {
            name: rem_name.clone(),
            kind: if removed {
                ChangeKind::Removed
            } else {
                ChangeKind::Unchanged
            },
            before,
            after: Value::Null,
            managed_keys: ManagedKeys::default(),
        });
    }

    // 仅当被覆盖的条目区间内部自身包含注释且用户未授权剥离时，才阻断写入
    if doc.lost_comments() && !allow_comment_strip {
        plan.blocked = Some(
            "目标条目内部含注释，自动重写会丢失该注释；确认后可允许去除注释再写入".to_string(),
        );
        return Ok(plan);
    }

    if !unresolved_vars.is_empty() {
        unresolved_vars.sort();
        unresolved_vars.dedup();
        let joined = unresolved_vars.join(", ");
        plan.warnings.push(format!(
            "{} 处密钥需内联（{}），客户端不支持变量展开或当前作用域禁止明文落盘",
            unresolved_vars.len(),
            joined
        ));
        plan.blocked = Some(format!(
            "未解析的内联密钥（{joined}）：{} 不支持环境变量引用或项目级配置禁止落盘明文密钥，已阻止写入",
            adapter.display_name()
        ));
        return Ok(plan);
    }

    plan.new_content = doc.render();
    let (old, new) = plan.display_texts();
    plan.line_diff = diff::line_diff(&old, &new);

    for e in entries {
        for platform in e.overrides.keys() {
            plan.warnings
                .push(format!("{}: 已应用 {} 平台覆盖", e.name, platform.as_str()));
        }
    }
    plan.warnings.dedup();

    Ok(plan)
}

/// 一个规划输入。
pub struct TargetInput<'a> {
    pub adapter: &'a dyn Adapter,
    pub scope: Scope,
    pub path: String,
    pub existing: Option<&'a str>,
}

/// 批量规划。
pub fn plan_files(
    targets: &[TargetInput<'_>],
    entries: &[McpEntry],
    mode: MergeMode,
    allow_comment_strip: bool,
) -> Result<Vec<FilePlan>> {
    targets
        .iter()
        .map(|t| {
            plan_file(
                t.adapter,
                t.path.clone(),
                t.scope,
                t.existing,
                entries,
                mode,
                allow_comment_strip,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters;
    use crate::model::Auth;
    use serde_json::json;

    fn github() -> McpEntry {
        let mut e = McpEntry::new_http("github", "https://api.githubcopilot.com/mcp/");
        e.auth = Some(Auth::env_ref("GITHUB_PAT"));
        e
    }

    #[test]
    fn new_file_plan_creates_root() {
        let a = adapters::by_id("claude-code").unwrap();
        let plan = plan_file(
            a,
            "/tmp/x.json",
            Scope::User,
            None,
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        assert!(!plan.existed);
        assert!(plan.has_write());
        assert_eq!(plan.changes[0].kind, ChangeKind::New);
        assert!(plan.new_content.contains("mcpServers"));
        assert!(plan.diff_text().contains("+"));
    }

    #[test]
    fn unchanged_plan_has_no_write() {
        let a = adapters::by_id("claude-code").unwrap();
        let first = plan_file(
            a,
            "/tmp/x.json",
            Scope::User,
            None,
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        let second = plan_file(
            a,
            "/tmp/x.json",
            Scope::User,
            Some(&first.new_content),
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        assert_eq!(second.changes[0].kind, ChangeKind::Unchanged);
        assert!(!second.has_write(), "幂等：内容一致时不应产生写入");
        assert!(second.diff_text().is_empty());
    }

    #[test]
    fn preserves_existing_other_entries() {
        let a = adapters::by_id("claude-code").unwrap();
        let existing = r#"{"mcpServers":{"mine":{"command":"node"}},"theme":"dark"}"#;
        let plan = plan_file(
            a,
            "/tmp/x.json",
            Scope::User,
            Some(existing),
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&plan.new_content).unwrap();
        assert_eq!(v["theme"], "dark", "无关配置必须保留");
        assert_eq!(
            v["mcpServers"]["mine"]["command"], "node",
            "用户自己的条目必须保留"
        );
        assert!(v["mcpServers"]["github"].is_object());
    }

    #[test]
    fn unsupported_scope_is_blocked() {
        let a = adapters::by_id("kimi-code").unwrap();
        let plan = plan_file(
            a,
            "/tmp/x.json",
            Scope::Project,
            None,
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        assert!(plan.blocked.is_some());
        assert!(!plan.has_write());
    }

    #[test]
    fn broken_json_is_blocked_not_rebuilt() {
        let a = adapters::by_id("claude-code").unwrap();
        let plan = plan_file(
            a,
            "/tmp/x.json",
            Scope::User,
            Some("{ this is not json"),
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        let reason = plan.blocked.clone().unwrap();
        assert!(reason.contains("解析失败"));
        assert_eq!(plan.new_content, plan.old_content, "解析失败时不得重建内容");
        assert!(!plan.has_write());
    }

    #[test]
    fn jsonc_preserves_outer_comments_and_blocks_only_interior_comments() {
        let a = adapters::by_id("opencode").unwrap();
        let outer_commented = "{\n  // 顶层注释不会丢失\n  \"mcp\": {\"servers\": {}}\n}";
        let plan = plan_file(
            a,
            "/tmp/o.jsonc",
            Scope::User,
            Some(outer_commented),
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        assert!(plan.blocked.is_none(), "外层注释无损保留，不应误阻断");
        assert!(plan.new_content.contains("// 顶层注释不会丢失"));
        assert!(plan.new_content.contains("github"));

        // 被覆盖的目标条目内部包含注释时，默认拒绝改写，显式开启 allow_comment_strip 后允许
        let interior_commented =
            "{\n  // 外部保留\n  \"mcp\": {\"servers\": {\"github\": { /* 内部注释 */ \"url\": \"old\" }}}\n}";
        let blocked = plan_file(
            a,
            "/tmp/o.jsonc",
            Scope::User,
            Some(interior_commented),
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        assert!(blocked.blocked.is_some(), "条目内部含注释应默认阻断");
        assert!(!blocked.has_write());

        let allowed = plan_file(
            a,
            "/tmp/o.jsonc",
            Scope::User,
            Some(interior_commented),
            &[github()],
            MergeMode::Merge,
            true,
        )
        .unwrap();
        assert!(allowed.blocked.is_none());
        assert!(allowed.new_content.contains("// 外部保留"));
        assert!(allowed.new_content.contains("https://api.githubcopilot.com/mcp/"));
    }

    #[test]
    fn jsonc_without_comments_is_fine() {
        let a = adapters::by_id("opencode").unwrap();
        let existing = r#"{"mcp":{"servers":{}}}"#;
        let plan = plan_file(
            a,
            "/tmp/o.json",
            Scope::User,
            Some(existing),
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        assert!(plan.blocked.is_none());
    }

    #[test]
    fn toml_plan_keeps_comments() {
        let a = adapters::by_id("codex").unwrap();
        let existing = "# 我的注释\n[mcp_servers.old]\ncommand = \"npx\"\n";
        let plan = plan_file(
            a,
            "/tmp/config.toml",
            Scope::User,
            Some(existing),
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        assert!(plan.new_content.contains("# 我的注释"));
        assert!(plan.new_content.contains("[mcp_servers.old]"));
        assert!(plan.new_content.contains("bearer_token_env_var"));
    }

    #[test]
    fn unresolved_inline_secret_blocks_write_and_emits_warning() {
        let a = adapters::by_id("workbuddy").unwrap();
        let plan = plan_file(
            a,
            "/tmp/w.json",
            Scope::User,
            None,
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        assert!(plan.warnings.iter().any(|w| w.contains("需内联")));
        assert!(plan.blocked.is_some(), "未解析内联密钥必须阻断，禁止写出哨兵字符串");
        assert!(!plan.has_write());
        assert!(!plan.new_content.contains("<需内联:"));
        let _ = json!({});
    }

    #[test]
    fn changed_count_matches() {
        let a = adapters::by_id("claude-code").unwrap();
        let existing = r#"{"mcpServers":{"github":{"url":"old"}}}"#;
        let plan = plan_file(
            a,
            "/tmp/x.json",
            Scope::User,
            Some(existing),
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        assert_eq!(plan.changed_count(), (0, 1));
    }
}
