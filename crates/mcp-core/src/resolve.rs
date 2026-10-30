//! 平台覆盖合并 + 变量插值 + 生效作用域解析。
//!
//! 适配器**不感知**平台差异：它们只接收已 Resolve 的条目。

use crate::interpolate::{interpolate_str, MissingVar, VarCtx};
use crate::model::{Catalog, McpEntry, Platform, Scope};
use std::collections::BTreeMap;

/// 应用平台覆盖，返回合并后的条目（不修改原条目）。
pub fn apply_platform(entry: &McpEntry, platform: Platform) -> McpEntry {
    let mut out = entry.clone();
    if let Some(ov) = entry.overrides.get(&platform) {
        if let Some(v) = &ov.command {
            out.command = Some(v.clone());
        }
        if let Some(v) = &ov.args {
            out.args = v.clone();
        }
        if let Some(v) = &ov.env {
            for (k, val) in v {
                out.env.insert(k.clone(), val.clone());
            }
        }
        if let Some(v) = &ov.cwd {
            out.cwd = Some(v.clone());
        }
        if let Some(v) = &ov.url {
            out.url = Some(v.clone());
        }
        if let Some(v) = &ov.headers {
            for (k, val) in v {
                out.headers.insert(k.clone(), val.clone());
            }
        }
    }
    // 合并后覆盖信息不再需要，避免下游误用
    out.overrides = BTreeMap::new();
    out
}

/// 平台覆盖 + 插值。返回条目与未解析变量列表。
pub fn resolve(entry: &McpEntry, platform: Platform, ctx: &VarCtx) -> (McpEntry, Vec<MissingVar>) {
    let mut out = apply_platform(entry, platform);
    let mut missing: Vec<MissingVar> = Vec::new();

    if let Some(cmd) = &out.command {
        out.command = Some(interpolate_str(cmd, ctx, &mut missing));
    }
    out.args = out
        .args
        .iter()
        .map(|a| interpolate_str(a, ctx, &mut missing))
        .collect();
    if let Some(cwd) = &out.cwd {
        out.cwd = Some(interpolate_str(cwd, ctx, &mut missing));
    }
    if let Some(url) = &out.url {
        out.url = Some(interpolate_str(url, ctx, &mut missing));
    }
    out.env = out
        .env
        .iter()
        .map(|(k, v)| (k.clone(), interpolate_str(v, ctx, &mut missing)))
        .collect();
    out.headers = out
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), interpolate_str(v, ctx, &mut missing)))
        .collect();

    // 去重（同一个变量可能在多处出现）
    missing.sort_by(|a, b| a.raw.cmp(&b.raw));
    missing.dedup_by(|a, b| a.raw == b.raw);

    (out, missing)
}

/// 生效作用域：条目自带优先，否则跟随全局默认。
pub fn effective_scope(entry: &McpEntry, global: Scope) -> Scope {
    entry.scope.unwrap_or(global)
}

/// 取某作用域下的条目（生效作用域匹配）。
pub fn entries_for_scope(entries: &[McpEntry], global: Scope, scope: Scope) -> Vec<McpEntry> {
    entries
        .iter()
        .filter(|e| effective_scope(e, global) == scope)
        .cloned()
        .collect()
}

/// 解析整个清单（逐条），返回条目与全部未解析变量。
pub fn resolve_catalog(
    catalog: &Catalog,
    platform: Platform,
    ctx: &VarCtx,
) -> (Vec<McpEntry>, Vec<MissingVar>) {
    let mut entries = Vec::with_capacity(catalog.entries.len());
    let mut all_missing = Vec::new();
    for e in &catalog.entries {
        let (r, m) = resolve(e, platform, ctx);
        entries.push(r);
        all_missing.extend(m);
    }
    all_missing.sort_by(|a, b| a.raw.cmp(&b.raw));
    all_missing.dedup_by(|a, b| a.raw == b.raw);
    (entries, all_missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EntryOverride, Transport};

    fn entry_with_override() -> McpEntry {
        let mut e = McpEntry::new_stdio(
            "stockdb",
            "python",
            vec!["-u".into(), "${PROJECT_ROOT}/s.py".into()],
        );
        e.env.insert("DB".into(), "${ENV:DB_URL}".into());
        let ov = EntryOverride {
            command: Some("C:/py/python.exe".into()),
            ..Default::default()
        };
        e.overrides.insert(Platform::Windows, ov);
        e
    }

    #[test]
    fn windows_override_replaces_command_but_keeps_args() {
        let e = entry_with_override();
        let merged = apply_platform(&e, Platform::Windows);
        assert_eq!(merged.command.as_deref(), Some("C:/py/python.exe"));
        assert_eq!(merged.args.len(), 2, "args 不应被清空");
    }

    #[test]
    fn linux_without_override_keeps_base() {
        let e = entry_with_override();
        let merged = apply_platform(&e, Platform::Linux);
        assert_eq!(merged.command.as_deref(), Some("python"));
    }

    #[test]
    fn resolve_interpolates_args_and_env() {
        let e = entry_with_override();
        let mut vars = BTreeMap::new();
        vars.insert("DB_URL".to_string(), "postgres://x".to_string());
        let ctx = VarCtx {
            home: Some("/home/dev".into()),
            project_root: Some("/proj".into()),
            vars,
            read_process_env: false,
        };
        let (r, missing) = resolve(&e, Platform::Linux, &ctx);
        assert_eq!(r.args[1], "/proj/s.py");
        assert_eq!(r.env.get("DB").unwrap(), "postgres://x");
        assert!(missing.is_empty());
    }

    #[test]
    fn resolve_reports_missing_env() {
        let e = entry_with_override();
        let ctx = VarCtx {
            home: None,
            project_root: Some("/proj".into()),
            vars: BTreeMap::new(),
            read_process_env: false,
        };
        let (_r, missing) = resolve(&e, Platform::Linux, &ctx);
        assert!(missing.iter().any(|m| m.name == "DB_URL"));
    }

    #[test]
    fn scope_inherits_or_overrides() {
        let mut e = McpEntry::new_stdio("x", "npx", vec![]);
        assert_eq!(effective_scope(&e, Scope::User), Scope::User);
        e.scope = Some(Scope::Project);
        assert_eq!(effective_scope(&e, Scope::User), Scope::Project);
        e.transport = Transport::Stdio;
    }
}
