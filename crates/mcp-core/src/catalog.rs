//! 客户清单（Catalog）解析与校验。
//!
//! 唯一事实来源：客户自维护的 YAML/JSON。不接公共 Registry。

use crate::error::{Error, Result};
use crate::model::{Catalog, McpEntry, Transport};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub entry: Option<String>,
    pub severity: Severity,
    pub message: String,
}

impl Issue {
    fn error(entry: Option<&str>, msg: impl Into<String>) -> Self {
        Self {
            entry: entry.map(|s| s.to_string()),
            severity: Severity::Error,
            message: msg.into(),
        }
    }
    fn warn(entry: Option<&str>, msg: impl Into<String>) -> Self {
        Self {
            entry: entry.map(|s| s.to_string()),
            severity: Severity::Warning,
            message: msg.into(),
        }
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tag = if self.is_error() { "错误" } else { "告警" };
        match &self.entry {
            Some(e) => write!(f, "[{tag}] {e}: {}", self.message),
            None => write!(f, "[{tag}] {}", self.message),
        }
    }
}

/// 解析清单文本（YAML 或 JSON，YAML 是 JSON 超集）。
pub fn parse(src: &str) -> Result<Catalog> {
    serde_yaml::from_str::<Catalog>(src).map_err(|e| {
        let location = e
            .location()
            .map(|l| format!("（行 {}，列 {}）", l.line(), l.column()))
            .unwrap_or_default();
        Error::Parse(format!("清单解析失败{location}，请检查语法和字段结构"))
    })
}

/// 序列化为 YAML（供 GUI 保存）。
pub fn to_yaml(catalog: &Catalog) -> Result<String> {
    serde_yaml::to_string(catalog).map_err(|e| Error::Parse(format!("清单序列化失败：{e}")))
}

/// 校验清单，返回问题列表。
pub fn validate(catalog: &Catalog) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();

    if catalog.entries.is_empty() {
        issues.push(Issue::warn(None, "清单为空"));
    }

    for e in &catalog.entries {
        if e.name.trim().is_empty() {
            issues.push(Issue::error(None, "存在空名称条目"));
            continue;
        }
        if !seen.insert(e.name.as_str()) {
            issues.push(Issue::error(Some(&e.name), "名称重复，清单内必须唯一"));
        }
        if e.name.contains(char::is_whitespace) {
            issues.push(Issue::warn(
                Some(&e.name),
                "名称含空白字符，部分客户端可能不接受",
            ));
        }

        match e.transport {
            Transport::Stdio => {
                if e.command.as_deref().unwrap_or("").trim().is_empty() {
                    issues.push(Issue::error(Some(&e.name), "stdio 条目缺少 command"));
                }
                if e.url.is_some() {
                    issues.push(Issue::warn(Some(&e.name), "stdio 条目不应有 url，将被忽略"));
                }
            }
            Transport::Http | Transport::Sse => {
                if e.url.as_deref().unwrap_or("").trim().is_empty() {
                    issues.push(Issue::error(Some(&e.name), "远程条目缺少 url"));
                }
                if e.command.is_some() {
                    issues.push(Issue::warn(
                        Some(&e.name),
                        "远程条目不应有 command，将被忽略",
                    ));
                }
            }
        }

        // 明文密钥告警
        if let Some(crate::model::Auth::Bearer {
            token: crate::model::SecretSource::Literal(_),
        }) = &e.auth
        {
            issues.push(Issue::warn(
                Some(&e.name),
                "使用明文 token，建议改为 token_env 引用环境变量",
            ));
        }

        // 平台覆盖合法性
        for (platform, ov) in &e.overrides {
            if e.transport == Transport::Stdio && ov.url.is_some() {
                issues.push(Issue::warn(
                    Some(&e.name),
                    format!("{} 覆盖含 url，但条目是 stdio", platform.as_str()),
                ));
            }
            if ov.args.is_some() && ov.command.is_none() {
                issues.push(Issue::warn(
                    Some(&e.name),
                    format!(
                        "{} 覆盖只改了 args 未改 command，注意语义",
                        platform.as_str()
                    ),
                ));
            }
        }
    }

    issues
}

/// 解析 + 校验。存在 Error 级问题时返回 Err。
pub fn load(src: &str) -> Result<(Catalog, Vec<Issue>)> {
    let catalog = parse(src)?;
    let issues = validate(&catalog);
    if let Some(first) = issues.iter().find(|i| i.is_error()) {
        return Err(Error::Invalid(first.to_string()));
    }
    Ok((catalog, issues))
}

/// 按名称查找。
pub fn find<'a>(catalog: &'a Catalog, name: &str) -> Option<&'a McpEntry> {
    catalog.entries.iter().find(|e| e.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
version: 1
entries:
  - name: stockdb
    transport: stdio
    command: python
    args: ["-u", "${PROJECT_ROOT}/s.py"]
    env:
      STOCKDB_URL: ${ENV:STOCKDB_URL}
    overrides:
      windows:
        command: c:/py/python.exe
  - name: github
    transport: http
    url: https://api.githubcopilot.com/mcp/
    auth:
      type: bearer
      token_env: GITHUB_PAT
"#;

    #[test]
    fn parses_sample() {
        let (c, issues) = load(SAMPLE).unwrap();
        assert_eq!(c.entries.len(), 2);
        assert!(issues.iter().all(|i| !i.is_error()));
        let gh = find(&c, "github").unwrap();
        assert_eq!(gh.auth.as_ref().unwrap().env_name(), Some("GITHUB_PAT"));
    }

    #[test]
    fn rejects_duplicate_names() {
        let src = r#"
version: 1
entries:
  - { name: a, transport: stdio, command: npx }
  - { name: a, transport: stdio, command: npx }
"#;
        let err = load(src).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn rejects_stdio_without_command() {
        let src = r#"
version: 1
entries:
  - { name: a, transport: stdio }
"#;
        assert!(load(src).is_err());
    }

    #[test]
    fn rejects_remote_without_url() {
        let src = r#"
version: 1
entries:
  - { name: a, transport: http }
"#;
        assert!(load(src).is_err());
    }

    #[test]
    fn scope_inherit_keyword_accepted() {
        let src = r#"
version: 1
entries:
  - { name: a, transport: stdio, command: npx, scope: inherit }
  - { name: b, transport: stdio, command: npx, scope: project }
"#;
        let (c, _) = load(src).unwrap();
        assert_eq!(c.entries[0].scope, None);
        assert_eq!(c.entries[1].scope, Some(crate::model::Scope::Project));
    }

    #[test]
    fn plaintext_token_warns() {
        let src = r#"
version: 1
entries:
  - name: a
    transport: http
    url: https://x/mcp
    auth: { type: bearer, token: ghp_xxx }
"#;
        let (_c, issues) = load(src).unwrap();
        assert!(issues.iter().any(|i| i.message.contains("明文")));
    }
}
