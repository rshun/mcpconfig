//! 应用计划：把「清单条目 + 目标文件」编译成逐文件的变更方案。
//!
//! 纯函数：接收已有文本（可能是 `None`），返回**将要写入的内容**与差异，
//! 不碰文件系统。写盘由 `mcp-io` 负责。

use crate::adapters::{Adapter, Format};
use crate::diff::{self, DiffLine};
use crate::document::{merge_json, ConfigDoc};
pub use crate::document::{ChangeKind, MergeMode};
use crate::error::Result;
use crate::model::{McpEntry, Scope};
use serde_json::Value;

/// 单条目的变更明细。
#[derive(Debug, Clone, PartialEq)]
pub struct EntryChange {
    pub name: String,
    pub kind: ChangeKind,
    /// 现有内容（新增时为 None）
    pub before: Option<Value>,
    /// 本次将写入的内容（合并后的最终值）
    pub after: Value,
}

/// 一个目标文件的完整计划。
#[derive(Debug, Clone)]
pub struct FilePlan {
    pub client_id: String,
    pub client_name: String,
    pub scope: Scope,
    pub path: String,
    pub format: Format,
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

/// 规划单个文件。
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
    let path = path.into();
    let format = adapter.format_for_path(std::path::Path::new(&path));
    let existed = existing.is_some();
    let old_content = existing.unwrap_or("").to_string();

    let mut plan = FilePlan {
        client_id: adapter.id().to_string(),
        client_name: adapter.display_name().to_string(),
        scope,
        path,
        format,
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

    // JSONC 注释保护：一旦重写会丢注释，默认拒绝
    if matches!(format, Format::Jsonc)
        && crate::document::has_jsonc_features(&old_content)
        && !allow_comment_strip
    {
        plan.blocked = Some(
            "该文件含注释或尾逗号，自动重写会丢失注释；确认后可允许去除注释再写入".to_string(),
        );
        return Ok(plan);
    }

    let mut doc = match ConfigDoc::parse(format, old_content.trim_start_matches('\u{feff}')) {
        Ok(d) => d,
        Err(e) => {
            plan.blocked = Some(format!("现有文件解析失败，已跳过（不会重建）：{e}"));
            return Ok(plan);
        }
    };

    let root = adapter.root_key_for_path(scope, std::path::Path::new(&plan.path));
    let mut inline_secrets = 0usize;

    for entry in entries {
        let encoded = adapter.encode_entry(entry)?;
        inline_secrets += count_inline_placeholders(&encoded);

        let before = doc.get(&root, &entry.name);
        let kind = doc.upsert(&root, &entry.name, &encoded, mode)?;

        // 合并后的最终值（用于 UI 展示）
        let after = match (&before, kind, mode) {
            (Some(old), ChangeKind::Unchanged, _) => old.clone(),
            (Some(old), _, MergeMode::Merge) => merge_json(old, &encoded),
            _ => encoded.clone(),
        };

        plan.changes.push(EntryChange {
            name: entry.name.clone(),
            kind,
            before,
            after,
        });
    }

    plan.new_content = doc.render();
    let (old, new) = plan.display_texts();
    plan.line_diff = diff::line_diff(&old, &new);

    if inline_secrets > 0 {
        plan.warnings.push(format!(
            "{inline_secrets} 处密钥需内联（该客户端不支持变量展开），按 secret_policy 处理"
        ));
    }
    for e in entries {
        for platform in e.overrides.keys() {
            plan.warnings
                .push(format!("{}: 已应用 {} 平台覆盖", e.name, platform.as_str()));
        }
    }
    plan.warnings.dedup();

    Ok(plan)
}

/// 统计编码结果里「需内联」占位符数量。
fn count_inline_placeholders(v: &Value) -> usize {
    const MARK: &str = "<需内联:";
    match v {
        Value::String(s) => usize::from(s.contains(MARK)),
        Value::Array(a) => a.iter().map(count_inline_placeholders).sum(),
        Value::Object(m) => m.values().map(count_inline_placeholders).sum(),
        _ => 0,
    }
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
    fn jsonc_with_comments_is_blocked_by_default() {
        let a = adapters::by_id("opencode").unwrap();
        let existing = "{\n  // 注释\n  \"mcp\": {\"servers\": {}}\n}";
        let plan = plan_file(
            a,
            "/tmp/o.jsonc",
            Scope::User,
            Some(existing),
            &[github()],
            MergeMode::Merge,
            false,
        )
        .unwrap();
        assert!(plan.blocked.is_some(), "含注释应默认拒绝改写");

        let allowed = plan_file(
            a,
            "/tmp/o.jsonc",
            Scope::User,
            Some(existing),
            &[github()],
            MergeMode::Merge,
            true,
        )
        .unwrap();
        assert!(allowed.blocked.is_none());
        assert!(allowed.new_content.contains("github"));
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
    fn inline_secret_warning_emitted() {
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
