//! 配置库安全编辑会话：
//! 1. `load` 在离开后端前通过 `mcp_core::redact` 隐藏全部明文凭据，原始条目仅驻留 Rust 内存；
//! 2. `save` 校验 `revision_id` 与磁盘原文快照（CAS 乐观并发控制，防止静默覆盖外部并发编辑），
//!    并把未改动的 `[已隐藏]` 占位符精确回填为原始凭据后再落盘。

use crate::{App, AppError};
use mcp_core::catalog;
use mcp_core::redact::HIDDEN_MARKER;
use mcp_core::Catalog;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

// 严禁派生 Debug 或 Serialize：内部持有未脱敏原文与凭据。
struct CatalogSnapshot {
    revision_id: String,
    raw_text: Option<String>,
    raw_entries: BTreeMap<String, Value>,
}

/// 脱敏后的配置库视图（可安全通过 IPC 发往前端）。
#[derive(Debug, Clone)]
pub struct LoadedCatalog {
    pub exists: bool,
    pub revision_id: String,
    pub catalog: Value,
    pub issues: Vec<String>,
    pub entry_count: usize,
}

#[derive(Debug, Clone)]
pub struct SaveCatalogOutcome {
    pub revision_id: String,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct CatalogSessions {
    serial: u64,
    snapshots: BTreeMap<String, CatalogSnapshot>,
}

fn path_key(path: &Path) -> String {
    mcp_io::file_key(path).unwrap_or_else(|_| path.to_string_lossy().to_string())
}

impl CatalogSessions {
    fn next_revision(&mut self) -> String {
        self.serial += 1;
        format!("cat-rev-{}-{}", std::process::id(), self.serial)
    }

    /// 当外部受控操作（如 `create_catalog` 或 `commit_import`）更新磁盘清单后，刷新或清理旧快照。
    pub fn sync_path(&mut self, path: &Path) {
        let key = path_key(path);
        if self.snapshots.contains_key(&key) {
            let _ = self.capture_existing(path);
        }
    }

    fn capture_existing(&mut self, path: &Path) -> Result<(String, Option<(Catalog, Vec<String>)>), AppError> {
        let key = path_key(path);
        match std::fs::read_to_string(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let rev = self.next_revision();
                self.snapshots.insert(
                    key,
                    CatalogSnapshot {
                        revision_id: rev.clone(),
                        raw_text: None,
                        raw_entries: BTreeMap::new(),
                    },
                );
                Ok((rev, None))
            }
            Err(_) => Err(AppError::Other("无法读取配置库，请检查路径和权限".into())),
            Ok(raw_text) => {
                let (cat, issues) = catalog::load(&raw_text)?;
                let mut raw_entries = BTreeMap::new();
                for entry in &cat.entries {
                    let v = serde_json::to_value(entry)
                        .map_err(|e| AppError::Other(e.to_string()))?;
                    raw_entries.insert(entry.name.clone(), v);
                }
                let rev = self.next_revision();
                self.snapshots.insert(
                    key,
                    CatalogSnapshot {
                        revision_id: rev.clone(),
                        raw_text: Some(raw_text),
                        raw_entries,
                    },
                );
                let issue_strs = issues.into_iter().map(|i| i.to_string()).collect();
                Ok((rev, Some((cat, issue_strs))))
            }
        }
    }

    /// 读取并脱敏清单；原始凭据保存在会话快照中，绝不离开后端。
    pub fn load(&mut self, _app: &App, path: &Path) -> Result<LoadedCatalog, AppError> {
        let (revision_id, loaded) = self.capture_existing(path)?;
        let Some((cat, issues)) = loaded else {
            return Ok(LoadedCatalog {
                exists: false,
                revision_id,
                catalog: json!({"version": 1, "entries": []}),
                issues: Vec::new(),
                entry_count: 0,
            });
        };
        let entry_count = cat.entries.len();
        let redacted_entries: Vec<Value> = cat
            .entries
            .iter()
            .map(|e| {
                let raw = serde_json::to_value(e).unwrap_or(Value::Null);
                mcp_core::redact::config_entry(&raw)
            })
            .collect();
        Ok(LoadedCatalog {
            exists: true,
            revision_id,
            catalog: json!({
                "version": cat.version,
                "entries": redacted_entries,
            }),
            issues,
            entry_count,
        })
    }

    /// 校验修订版本与磁盘并发状态（CAS），回填未修改的 `[已隐藏]` 凭据并原子保存。
    pub fn save(
        &mut self,
        app: &App,
        path: &Path,
        mut incoming_catalog: Value,
        revision_id: Option<&str>,
        renames: Option<&BTreeMap<String, String>>,
    ) -> Result<SaveCatalogOutcome, AppError> {
        let key = path_key(path);
        let snapshot = self.snapshots.get(&key);

        if let Some(expected_rev) = revision_id {
            let Some(snap) = snapshot else {
                return Err(AppError::Other(
                    "配置库会话不存在或已失效，请重新加载后再保存".into(),
                ));
            };
            if snap.revision_id != expected_rev {
                return Err(AppError::Other(
                    "配置库修订号已过期，可能已被其他窗口或导入操作更新，请刷新后重试".into(),
                ));
            }
        }

        if let Some(entries) = incoming_catalog
            .as_object_mut()
            .and_then(|obj| obj.get_mut("entries"))
            .and_then(Value::as_array_mut)
        {
            for entry_val in entries.iter_mut() {
                let Some(name) = entry_val
                    .as_object()
                    .and_then(|o| o.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                else {
                    continue;
                };
                let orig_name = renames
                    .and_then(|m| m.get(&name))
                    .map(String::as_str)
                    .unwrap_or(&name);
                let orig_raw = snapshot.and_then(|s| s.raw_entries.get(orig_name));
                rehydrate_entry(entry_val, orig_raw)?;
            }
        }

        let cat: Catalog = serde_json::from_value(incoming_catalog)
            .map_err(|_| AppError::Other("清单字段结构无效，请检查输入".into()))?;

        let expected_disk = snapshot.map(|s| s.raw_text.as_deref());
        let warnings = app
            .save_catalog_checked(path, &cat, expected_disk)
            .map_err(|err| match err {
                AppError::Io(mcp_io::IoError::Conflict(_)) => AppError::Other(
                    "配置库文件在磁盘上已被外部修改，已拒绝覆盖；请重新加载后再保存".into(),
                ),
                other => other,
            })?;

        let (new_rev, _) = self.capture_existing(path)?;
        Ok(SaveCatalogOutcome {
            revision_id: new_rev,
            warnings,
        })
    }
}

fn rehydrate_entry(incoming: &mut Value, orig_raw: Option<&Value>) -> Result<(), AppError> {
    let orig_redacted = orig_raw.map(mcp_core::redact::config_entry);
    rehydrate_node(incoming, orig_raw, orig_redacted.as_ref(), false)
}

fn rehydrate_node(
    incoming: &mut Value,
    orig_raw: Option<&Value>,
    orig_redacted: Option<&Value>,
    is_args: bool,
) -> Result<(), AppError> {
    match incoming {
        Value::Object(inc_map) => {
            let raw_map = orig_raw.and_then(Value::as_object);
            let red_map = orig_redacted.and_then(Value::as_object);
            for (k, inc_v) in inc_map.iter_mut() {
                rehydrate_node(
                    inc_v,
                    raw_map.and_then(|m| m.get(k)),
                    red_map.and_then(|m| m.get(k)),
                    k == "args",
                )?;
            }
            Ok(())
        }
        Value::Array(inc_arr) => {
            let raw_arr = orig_raw.and_then(Value::as_array);
            let red_arr = orig_redacted.and_then(Value::as_array);
            let inc_snapshot = inc_arr.clone();
            for (i, inc_v) in inc_arr.iter_mut().enumerate() {
                if inc_v.as_str() == Some(HIDDEN_MARKER) {
                    if let (Some(r_arr), Some(d_arr)) = (raw_arr, red_arr) {
                        // 1. 同位置匹配（若为 args 且有前驱参数，要求前驱参数名一致）
                        let same_slot_ok = d_arr.get(i).and_then(Value::as_str) == Some(HIDDEN_MARKER)
                            && (!is_args || i == 0 || inc_snapshot.get(i - 1) == r_arr.get(i - 1));
                        if same_slot_ok {
                            *inc_v = r_arr[i].clone();
                            continue;
                        }
                        // 2. args 中前序参数增删但敏感参数名（如 `--password`）唯一匹配
                        if is_args && i > 0 {
                            let prev_flag = &inc_snapshot[i - 1];
                            let matches: Vec<&Value> = r_arr
                                .iter()
                                .enumerate()
                                .filter(|&(j, _)| {
                                    j > 0
                                        && &r_arr[j - 1] == prev_flag
                                        && d_arr.get(j).and_then(Value::as_str)
                                            == Some(HIDDEN_MARKER)
                                })
                                .map(|(_, val)| val)
                                .collect();
                            if matches.len() == 1 {
                                *inc_v = matches[0].clone();
                                continue;
                            }
                        }
                    }
                    return Err(AppError::Other(
                        "存在未填写的已隐藏凭据参数，请重新输入真实凭据或使用 ${ENV:VAR} 引用".into(),
                    ));
                }
                rehydrate_node(
                    inc_v,
                    raw_arr.and_then(|a| a.get(i)),
                    red_arr.and_then(|a| a.get(i)),
                    false,
                )?;
            }
            Ok(())
        }
        Value::String(s) if s == HIDDEN_MARKER => {
            if orig_redacted.and_then(Value::as_str) == Some(HIDDEN_MARKER) {
                if let Some(raw_val) = orig_raw {
                    *incoming = raw_val.clone();
                    return Ok(());
                }
            }
            Err(AppError::Other(
                "存在未填写的已隐藏凭据字段，请重新输入真实凭据或改为环境变量引用".into(),
            ))
        }
        _ => Ok(()),
    }
}
