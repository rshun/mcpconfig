use crate::AppError;
use mcp_core::{
    diff,
    document::ConfigDoc,
    plan::{FilePlan, MergeMode},
};
use std::collections::BTreeMap;
use std::path::Path;

/// 文件是写入事务边界。共享一个文件的 Agent 只生成一次写入和一次备份。
pub fn coalesce(plans: Vec<FilePlan>) -> Result<Vec<FilePlan>, AppError> {
    let mut groups: BTreeMap<String, Vec<FilePlan>> = BTreeMap::new();
    for plan in plans {
        groups
            .entry(mcp_io::file_key(Path::new(&plan.path))?)
            .or_default()
            .push(plan);
    }
    let mut result = Vec::new();
    for group in groups.into_values() {
        if group.len() == 1 {
            result.extend(group);
            continue;
        }
        let mut merged = group[0].clone();
        merged.client_id = group
            .iter()
            .map(|p| p.client_id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        merged.client_name = group
            .iter()
            .map(|p| p.client_name.as_str())
            .collect::<Vec<_>>()
            .join(" / ");
        if group.iter().any(|p| {
            p.blocked.is_some() || p.format != merged.format || p.old_content != merged.old_content
        }) {
            merged.blocked =
                Some("共享配置文件存在被阻止的计划、格式冲突或读取期间变化；整个文件未写入".into());
            result.push(merged);
            continue;
        }
        let mut doc = ConfigDoc::parse(
            merged.format,
            merged.old_content.trim_start_matches('\u{feff}'),
        )?;
        let mut changes = BTreeMap::new();
        let mut conflict = false;
        for plan in &group {
            let root = &plan.root_key;
            for change in &plan.changes {
                let key = (root.clone(), change.name.clone());
                if let Some(previous) = changes.get(&key) {
                    let previous: &mcp_core::plan::EntryChange = previous;
                    if previous.after != change.after {
                        conflict = true;
                    }
                } else {
                    changes.insert(key, change.clone());
                }
            }
        }
        if conflict {
            merged.blocked =
                Some("多个 Agent 对共享文件的同名 MCP 要求不同内容；请调整选择后重试".into());
        } else {
            for ((root, name), change) in &changes {
                doc.upsert(root, name, &change.after, MergeMode::Replace)?;
            }
            merged.new_content = doc.render();
            merged.changes = changes.into_values().collect();
            merged.warnings = group.iter().flat_map(|p| p.warnings.clone()).collect();
            merged
                .warnings
                .push("多个 Agent 共享此配置文件，已合并为一次写入".into());
            merged.warnings.push(
                "读取同一 MCP 容器的 Agent 会共同看到这些条目，无法在共享容器内隔离选择".into(),
            );
            let (old, new) = merged.display_texts();
            merged.line_diff = diff::line_diff(&old, &new);
        }
        result.push(merged);
    }
    Ok(result)
}
