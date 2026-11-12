//! MCP 条目按需自检（Handshake Probe）：
//! - 对 `stdio` 条目：拉起受控子进程（Windows 隐藏控制台窗口），在严格超时内执行
//!   JSON-RPC 2.0 `initialize` + `tools/list` 握手并立即回收子进程；
//! - 对 `http` / `sse` 条目：离线校验 URL 结构与认证环境变量就绪状态（零外部依赖）；
//! - 全程严禁在返回摘要中包含任何环境变量值或明文凭据。

use crate::App;
use mcp_core::interpolate::{interpolate_str, MissingVar};
use mcp_core::model::{Auth, McpEntry, Platform, SecretSource, Transport};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    pub ok: bool,
    pub kind: String,
    pub summary: String,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub tools: Vec<String>,
    pub missing_vars: Vec<String>,
    pub duration_ms: u64,
}

fn resolve_executable(cmd: &str, platform: Platform) -> String {
    let trimmed = cmd.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || Path::new(trimmed).is_file()
    {
        return trimmed.to_string();
    }
    let Some(path_os) = std::env::var_os("PATH") else {
        return trimmed.to_string();
    };
    let exts: &[&str] = if platform == Platform::Windows {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    for dir in std::env::split_paths(&path_os) {
        for ext in exts {
            let cand: PathBuf = dir.join(format!("{trimmed}{ext}"));
            if cand.is_file() {
                return cand.display().to_string();
            }
        }
    }
    trimmed.to_string()
}

fn valid_http_url(url: &str) -> bool {
    let trimmed = url.trim();
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"));
    let Some(rest) = rest else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !host.chars().any(char::is_whitespace)
}

impl App {
    /// 按需探测单个 MCP 条目的可用性（不会修改任何配置文件）。
    pub fn probe_entry(&self, raw_entry: &McpEntry) -> ProbeReport {
        let started = Instant::now();
        let ctx = self.var_ctx();

        // 1. 叠加当前平台覆盖
        let mut entry = raw_entry.clone();
        if let Some(ov) = entry.overrides.get(&self.platform).cloned() {
            if let Some(v) = ov.command {
                entry.command = Some(v);
            }
            if let Some(v) = ov.args {
                entry.args = v;
            }
            if let Some(v) = ov.env {
                for (k, val) in v {
                    entry.env.insert(k, val);
                }
            }
            if let Some(v) = ov.cwd {
                entry.cwd = Some(v);
            }
            if let Some(v) = ov.url {
                entry.url = Some(v);
            }
            if let Some(v) = ov.headers {
                for (k, val) in v {
                    entry.headers.insert(k, val);
                }
            }
            if let Some(v) = ov.auth {
                entry.auth = Some(v);
            }
            if let Some(v) = ov.timeout_seconds {
                entry.timeout_seconds = Some(v);
            }
        }

        let kind = match entry.transport {
            Transport::Stdio => "stdio",
            Transport::Http => "http",
            Transport::Sse => "sse",
        }
        .to_string();

        // 2. 插值解析并收集缺失变量（仅记录变量名，绝不记录值）
        let mut missing: Vec<MissingVar> = Vec::new();
        let command = entry
            .command
            .as_deref()
            .map(|s| interpolate_str(s, &ctx, &mut missing));
        let args: Vec<String> = entry
            .args
            .iter()
            .map(|s| interpolate_str(s, &ctx, &mut missing))
            .collect();
        let cwd = entry
            .cwd
            .as_deref()
            .map(|s| interpolate_str(s, &ctx, &mut missing));
        let url = entry
            .url
            .as_deref()
            .map(|s| interpolate_str(s, &ctx, &mut missing));
        let mut resolved_env = BTreeMap::new();
        for (k, v) in &entry.env {
            resolved_env.insert(k.clone(), interpolate_str(v, &ctx, &mut missing));
        }
        for v in entry.headers.values() {
            let _ = interpolate_str(v, &ctx, &mut missing);
        }
        if let Some(auth) = &entry.auth {
            match auth {
                Auth::Bearer {
                    token: SecretSource::EnvRef(var),
                }
                | Auth::Header {
                    value: SecretSource::EnvRef(var),
                    ..
                } => {
                    if ctx.lookup_var(var).is_none() {
                        missing.push(MissingVar {
                            raw: format!("${{ENV:{var}}}"),
                            name: var.clone(),
                        });
                    }
                }
                _ => {}
            }
        }

        let mut missing_vars: Vec<String> = missing
            .into_iter()
            .map(|m| if m.name.is_empty() { m.raw } else { m.name })
            .collect();
        missing_vars.sort();
        missing_vars.dedup();

        // 3. 远程传输校验
        if matches!(entry.transport, Transport::Http | Transport::Sse) {
            let duration_ms = started.elapsed().as_millis() as u64;
            let Some(u) = url.as_deref().filter(|s| !s.trim().is_empty()) else {
                return ProbeReport {
                    ok: false,
                    kind,
                    summary: "未填写远程服务 URL".into(),
                    server_name: None,
                    server_version: None,
                    tools: Vec::new(),
                    missing_vars,
                    duration_ms,
                };
            };
            if !valid_http_url(u) {
                return ProbeReport {
                    ok: false,
                    kind,
                    summary: "远程 URL 格式无效，请使用 http:// 或 https:// 地址".into(),
                    server_name: None,
                    server_version: None,
                    tools: Vec::new(),
                    missing_vars,
                    duration_ms,
                };
            }
            if !missing_vars.is_empty() {
                return ProbeReport {
                    ok: false,
                    kind,
                    summary: format!("检测到未就绪的环境变量：{}", missing_vars.join(", ")),
                    server_name: None,
                    server_version: None,
                    tools: Vec::new(),
                    missing_vars,
                    duration_ms,
                };
            }
            return ProbeReport {
                ok: true,
                kind,
                summary: "远程地址与认证环境变量已就绪（连接由目标 Agent 运行时建立）".into(),
                server_name: None,
                server_version: None,
                tools: Vec::new(),
                missing_vars,
                duration_ms,
            };
        }

        // 4. Stdio 本地子进程 JSON-RPC 2.0 握手探测
        let Some(cmd_raw) = command.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            return ProbeReport {
                ok: false,
                kind,
                summary: "未填写启动命令（command）".into(),
                server_name: None,
                server_version: None,
                tools: Vec::new(),
                missing_vars,
                duration_ms: started.elapsed().as_millis() as u64,
            };
        };

        if !missing_vars.is_empty() {
            return ProbeReport {
                ok: false,
                kind,
                summary: format!(
                    "缺少本机环境变量（{}），请配置后再执行启动握手",
                    missing_vars.join(", ")
                ),
                server_name: None,
                server_version: None,
                tools: Vec::new(),
                missing_vars,
                duration_ms: started.elapsed().as_millis() as u64,
            };
        }

        let program = resolve_executable(cmd_raw, self.platform);
        let mut cmd_builder = Command::new(&program);
        cmd_builder
            .args(&args)
            .envs(&resolved_env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        if let Some(dir) = cwd.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let dir_path = Path::new(dir);
            if !dir_path.is_dir() {
                return ProbeReport {
                    ok: false,
                    kind,
                    summary: format!("工作目录不存在：{dir}"),
                    server_name: None,
                    server_version: None,
                    tools: Vec::new(),
                    missing_vars,
                    duration_ms: started.elapsed().as_millis() as u64,
                };
            }
            cmd_builder.current_dir(dir_path);
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd_builder.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = match cmd_builder.spawn() {
            Ok(c) => c,
            Err(e) => {
                let reason = if e.kind() == std::io::ErrorKind::NotFound {
                    format!("未找到可执行命令 `{cmd_raw}`，请检查 PATH 或填写绝对路径")
                } else {
                    format!("启动 `{cmd_raw}` 失败（{}）", e.kind())
                };
                return ProbeReport {
                    ok: false,
                    kind,
                    summary: reason,
                    server_name: None,
                    server_version: None,
                    tools: Vec::new(),
                    missing_vars,
                    duration_ms: started.elapsed().as_millis() as u64,
                };
            }
        };

        let mut child_stdin = child.stdin.take();
        let child_stdout = child.stdout.take();
        let timeout = Duration::from_secs(entry.timeout_seconds.unwrap_or(4).clamp(2, 10));

        let (tx, rx) = mpsc::channel::<Result<(Option<String>, Option<String>, Vec<String>), String>>();

        std::thread::spawn(move || {
            let outcome = (|| -> Result<(Option<String>, Option<String>, Vec<String>), String> {
                let Some(mut stdin) = child_stdin.take() else {
                    return Err("无法打开子进程标准输入".into());
                };
                let Some(stdout) = child_stdout else {
                    return Err("无法读取子进程标准输出".into());
                };
                let mut reader = BufReader::new(stdout);

                let init_req = json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {},
                        "clientInfo": {
                            "name": "mcp-hub",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                });
                writeln!(stdin, "{init_req}").map_err(|_| "写入 initialize 请求失败".to_string())?;
                stdin.flush().map_err(|_| "刷新标准输入失败".to_string())?;

                let mut server_name: Option<String> = None;
                let mut server_version: Option<String> = None;
                let mut got_init = false;
                let mut line = String::new();

                for _ in 0..32 {
                    line.clear();
                    let n = reader
                        .read_line(&mut line)
                        .map_err(|_| "读取 MCP 响应流失败".to_string())?;
                    if n == 0 {
                        break;
                    }
                    let trimmed = line.trim();
                    if !trimmed.starts_with('{') {
                        continue;
                    }
                    let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                        continue;
                    };
                    if msg.get("id").and_then(Value::as_i64) == Some(1) {
                        if let Some(err_obj) = msg.get("error") {
                            let code = err_obj.get("code").and_then(Value::as_i64).unwrap_or(0);
                            return Err(format!("MCP initialize 返回错误（code {code}）"));
                        }
                        if let Some(info) = msg.get("result").and_then(|r| r.get("serverInfo")) {
                            server_name = info
                                .get("name")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                            server_version = info
                                .get("version")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                        }
                        got_init = true;
                        break;
                    }
                }

                if !got_init {
                    return Err("进程已退出且未返回有效的 MCP initialize 响应".into());
                }

                // 尝试读取 tools/list（若服务端未实现 tools 能力，也不算握手失败）
                let initialized_notif = json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/initialized"
                });
                let list_tools_req = json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/list",
                    "params": {}
                });
                let mut tools = Vec::new();
                if writeln!(stdin, "{initialized_notif}").is_ok()
                    && writeln!(stdin, "{list_tools_req}").is_ok()
                    && stdin.flush().is_ok()
                {
                    for _ in 0..32 {
                        line.clear();
                        let Ok(n) = reader.read_line(&mut line) else {
                            break;
                        };
                        if n == 0 {
                            break;
                        }
                        let trimmed = line.trim();
                        if !trimmed.starts_with('{') {
                            continue;
                        }
                        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                            continue;
                        };
                        if msg.get("id").and_then(Value::as_i64) == Some(2) {
                            if let Some(arr) = msg
                                .get("result")
                                .and_then(|r| r.get("tools"))
                                .and_then(Value::as_array)
                            {
                                for item in arr {
                                    if let Some(name) = item.get("name").and_then(Value::as_str) {
                                        tools.push(name.to_string());
                                    }
                                }
                            }
                            break;
                        }
                    }
                }

                Ok((server_name, server_version, tools))
            })();
            let _ = tx.send(outcome);
        });

        let recv = rx.recv_timeout(timeout);
        let _ = child.kill();
        let _ = child.wait();
        let duration_ms = started.elapsed().as_millis() as u64;

        match recv {
            Ok(Ok((server_name, server_version, tools))) => {
                let title = match (&server_name, &server_version) {
                    (Some(n), Some(v)) if !v.is_empty() => format!("{n} v{v}"),
                    (Some(n), _) => n.clone(),
                    _ => entry.name.clone(),
                };
                let summary = if tools.is_empty() {
                    format!("MCP 握手成功 · {title}（{duration_ms} ms）")
                } else {
                    format!(
                        "MCP 握手成功 · {title} · 发现 {} 个工具（{duration_ms} ms）",
                        tools.len()
                    )
                };
                ProbeReport {
                    ok: true,
                    kind,
                    summary,
                    server_name,
                    server_version,
                    tools,
                    missing_vars,
                    duration_ms,
                }
            }
            Ok(Err(msg)) => ProbeReport {
                ok: false,
                kind,
                summary: msg,
                server_name: None,
                server_version: None,
                tools: Vec::new(),
                missing_vars,
                duration_ms,
            },
            Err(mpsc::RecvTimeoutError::Timeout) => ProbeReport {
                ok: false,
                kind,
                summary: format!(
                    "等待 MCP initialize 握手超时（>{}s），可能是首次下载依赖或命令阻塞",
                    timeout.as_secs()
                ),
                server_name: None,
                server_version: None,
                tools: Vec::new(),
                missing_vars,
                duration_ms,
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => ProbeReport {
                ok: false,
                kind,
                summary: "子进程提前退出，未完成 MCP 握手".into(),
                server_name: None,
                server_version: None,
                tools: Vec::new(),
                missing_vars,
                duration_ms,
            },
        }
    }
}
