//! 变量插值：`${ENV:NAME}`、`${HOME}`、`${PROJECT_ROOT}`。
//!
//! 只做字符串替换，不关心字节码或平台差异；由 Resolver 在编码前调用。

use std::collections::BTreeMap;

/// 插值时可见的上下文。
#[derive(Debug, Clone, Default)]
pub struct VarCtx {
    pub home: Option<String>,
    pub project_root: Option<String>,
    /// 显式注入的变量（优先级高于进程环境）。
    pub vars: BTreeMap<String, String>,
    /// 是否回退到进程环境变量。
    pub read_process_env: bool,
}

impl VarCtx {
    /// 读取真实环境变量（CLI / GUI 默认行为）。
    pub fn from_process(project_root: Option<String>) -> Self {
        Self {
            home: std::env::var("HOME")
                .ok()
                .or_else(|| std::env::var("USERPROFILE").ok()),
            project_root,
            vars: BTreeMap::new(),
            read_process_env: true,
        }
    }

    /// 查询环境变量（显式 `vars` 优先，其次按 `read_process_env` 回退进程环境）。
    pub fn lookup_var(&self, key: &str) -> Option<String> {
        self.lookup(key)
    }

    fn lookup(&self, key: &str) -> Option<String> {
        if let Some(v) = self.vars.get(key) {
            return Some(v.clone());
        }
        if self.read_process_env {
            return std::env::var(key).ok();
        }
        None
    }
}

/// 未解析的变量（供 GUI 告警）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingVar {
    /// 原始占位符，例如 `${ENV:GITHUB_PAT}`。
    pub raw: String,
    /// 变量名，例如 `GITHUB_PAT`。
    pub name: String,
}

/// 对单个字符串插值。无法解析的占位符**原样保留**，同时记录到 missing。
pub fn interpolate_str(input: &str, ctx: &VarCtx, missing: &mut Vec<MissingVar>) -> String {
    interpolate_scoped(input, ctx, true, None, missing)
}

/// 面向客户端与作用域的分阶段插值：
/// - 路径宏（`${HOME}` / `${HOME_DIR}` / `${PROJECT_ROOT}` / `${CWD}`）始终按 `ctx` 展开；
/// - 环境变量（`${ENV:VAR}` / `${env:VAR}` / `${VAR}`）：
///   - 当 `inline_env == true`（`User` / `Local` 作用域）时，若 `ctx` 可查到值则展开；若缺失且目标支持 `env_dialect`，翻译为目标客户端原生语法（如 `${VAR}` / `{env:VAR}` / `${env:VAR}`），同时记录到 `missing`；
///   - 当 `inline_env == false`（`Project` 作用域，受 Git 跟踪）时，**绝不内联宿主进程环境变量明文**；若目标支持 `env_dialect` 则翻译为目标客户端原生引用语法，否则保留 `${ENV:VAR}` 原样。
pub fn interpolate_scoped(
    input: &str,
    ctx: &VarCtx,
    inline_env: bool,
    env_dialect: Option<(&str, &str)>,
    missing: &mut Vec<MissingVar>,
) -> String {
    if !input.contains("${") {
        return input.to_string();
    }

    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // 找匹配的 }
            if let Some(rel_end) = input[i + 2..].find('}') {
                let end = i + 2 + rel_end;
                let token = input[i + 2..end].trim();
                let raw = &input[i..=end];

                // 1. 编译期路径宏（非环境变量）
                if matches!(token, "HOME" | "HOME_DIR" | "PROJECT_ROOT" | "CWD") {
                    let path_val = match token {
                        "HOME" | "HOME_DIR" => ctx.home.as_deref(),
                        _ => ctx.project_root.as_deref(),
                    };
                    match path_val {
                        Some(v) => out.push_str(v),
                        None => {
                            out.push_str(raw);
                            missing.push(MissingVar {
                                raw: raw.to_string(),
                                name: token.to_string(),
                            });
                        }
                    }
                    i = end + 1;
                    continue;
                }

                // 2. 运行期环境变量引用（${ENV:VAR} / ${env:VAR} / ${VAR}）
                let env_var = token
                    .strip_prefix("ENV:")
                    .or_else(|| token.strip_prefix("env:"))
                    .unwrap_or(token)
                    .trim();

                if env_var.is_empty() {
                    out.push_str(raw);
                    missing.push(MissingVar {
                        raw: raw.to_string(),
                        name: String::new(),
                    });
                    i = end + 1;
                    continue;
                }

                if inline_env {
                    match ctx.lookup(env_var) {
                        Some(value) => out.push_str(&value),
                        None => {
                            if let Some((prefix, suffix)) = env_dialect {
                                out.push_str(prefix);
                                out.push_str(env_var);
                                out.push_str(suffix);
                            } else {
                                out.push_str(raw);
                            }
                            missing.push(MissingVar {
                                raw: raw.to_string(),
                                name: env_var.to_string(),
                            });
                        }
                    }
                } else {
                    // Scope::Project：禁止把宿主进程环境变量明文落入受版本控制的项目文件
                    if let Some((prefix, suffix)) = env_dialect {
                        out.push_str(prefix);
                        out.push_str(env_var);
                        out.push_str(suffix);
                    } else {
                        out.push_str(raw);
                        if ctx.lookup(env_var).is_none() {
                            missing.push(MissingVar {
                                raw: raw.to_string(),
                                name: env_var.to_string(),
                            });
                        }
                    }
                }

                i = end + 1;
                continue;
            }
        }
        // 逐字符推进（保证 UTF-8 安全）
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&input[i..i + ch_len]);
        i += ch_len;
    }

    out
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> VarCtx {
        let mut vars = BTreeMap::new();
        vars.insert("GITHUB_PAT".to_string(), "ghp_abc".to_string());
        VarCtx {
            home: Some("/home/dev".into()),
            project_root: Some("/work/proj".into()),
            vars,
            read_process_env: false,
        }
    }

    #[test]
    fn replaces_env() {
        let mut m = vec![];
        let out = interpolate_str("Bearer ${ENV:GITHUB_PAT}", &ctx(), &mut m);
        assert_eq!(out, "Bearer ghp_abc");
        assert!(m.is_empty());
    }

    #[test]
    fn replaces_home_and_root() {
        let mut m = vec![];
        let out = interpolate_str("${HOME}/x:${PROJECT_ROOT}", &ctx(), &mut m);
        assert_eq!(out, "/home/dev/x:/work/proj");
        assert!(m.is_empty());
    }

    #[test]
    fn keeps_missing_placeholder() {
        let mut m = vec![];
        let out = interpolate_str("${ENV:NOPE}", &ctx(), &mut m);
        assert_eq!(out, "${ENV:NOPE}");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].name, "NOPE");
    }

    #[test]
    fn no_placeholder_is_noop() {
        let mut m = vec![];
        assert_eq!(interpolate_str("plain", &ctx(), &mut m), "plain");
        assert!(m.is_empty());
    }

    #[test]
    fn handles_utf8_around_placeholder() {
        let mut m = vec![];
        let out = interpolate_str("路径：${HOME}/数据库", &ctx(), &mut m);
        assert_eq!(out, "路径：/home/dev/数据库");
    }
}
