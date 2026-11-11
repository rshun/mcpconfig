import { useEffect, useMemo, useState } from "react";
import type { Scope } from "../types";
import { Banner, Empty, Field } from "../components/ui";

interface Entry {
  name: string;
  description?: string;
  transport: "stdio" | "http" | "sse";
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  cwd?: string;
  url?: string;
  headers?: Record<string, string>;
  auth?:
    | { type: "bearer"; token_env?: string; token?: string }
    | { type: "header" | "api_key"; header: string; prefix?: string; value_env?: string; value?: string };
  scope?: "inherit" | Scope;
  tags?: string[];
  overrides?: unknown;
  extensions?: Record<string, Record<string, unknown>>;
  enabled?: boolean;
  timeout_seconds?: number;
}

const SCOPE_OPTIONS: { value: string; label: string }[] = [
  { value: "inherit", label: "跟随全局" },
  { value: "user", label: "用户级" },
  { value: "project", label: "项目级" },
  { value: "local", label: "本地级" },
];

function pairToText(m?: Record<string, string>) {
  if (!m) return "";
  return Object.entries(m)
    .map(([k, v]) => `${k}=${v}`)
    .join(", ");
}
function textToPair(s: string): Record<string, string> | undefined {
  const out: Record<string, string> = {};
  s.split(",").forEach((kv) => {
    const [k, ...rest] = kv.split("=");
    if (k.trim()) out[k.trim()] = rest.join("=").trim();
  });
  return Object.keys(out).length ? out : undefined;
}

export function CatalogPage({
  entries,
  issues,
  onSave,
  onDirty,
}: {
  entries: Entry[] | null;
  issues: string[];
  onSave: (entries: Entry[]) => Promise<string[]>;
  onDirty: (dirty: boolean) => void;
}) {
  const [list, setList] = useState<Entry[]>(entries ?? []);
  const [cur, setCur] = useState(0);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => {
    setList(entries ?? []);
    setCur(0);
  }, [entries]);

  const entry = list[cur];
  const dirty = useMemo(() => JSON.stringify(list) !== JSON.stringify(entries ?? []), [list, entries]);

  useEffect(() => {
    onDirty(dirty);
  }, [dirty, onDirty]);

  function patch(p: Partial<Entry>) {
    setList((prev) => prev.map((e, i) => (i === cur ? { ...e, ...p } : e)));
  }

  function addEntry() {
    const n = `new-${list.length + 1}`;
    setList([...list, { name: n, transport: "stdio", command: "npx", scope: "inherit" }]);
    setCur(list.length);
  }

  function removeEntry() {
    if (!entry) return;
    const next = list.filter((_, i) => i !== cur);
    setList(next);
    setCur(Math.max(0, Math.min(cur, next.length - 1)));
  }

  async function save() {
    setSaving(true);
    setMsg(null);
    try {
      const warnings = await onSave(list);
      setMsg(["已保存到清单文件", ...warnings].join("；"));
      if (warnings.length === 0) setTimeout(() => setMsg(null), 2500);
    } catch (e) {
      setMsg(`保存失败：${String(e)}`);
    } finally {
      setSaving(false);
    }
  }

  if (!entries) return <Empty title="正在加载清单…" />;

  return (
    <>
      <h1>MCP配置</h1>
      <p className="sub">
        清单是唯一事实来源，所有客户端的配置都由它生成。改动会立即影响「客户端配置」页。
      </p>

      {issues.length > 0 && (
        <Banner tone="warn">
          <div>
            {issues.map((i) => (
              <div key={i}>{i}</div>
            ))}
          </div>
        </Banner>
      )}
      {msg && <Banner tone={msg.includes("失败") ? "danger" : "ok"}>{msg}</Banner>}

      <div className="split">
        <div>
          <div className="list">
            <button className="list-item" onClick={addEntry} style={{ color: "var(--accent)" }}>
              ＋ 新增配置
            </button>
            {list.map((e, i) => (
              <button
                key={`${e.name}-${i}`}
                className={`list-item${i === cur ? " active" : ""}`}
                onClick={() => setCur(i)}
              >
                <div className="nm">{e.name || "(未命名)"}</div>
                <div className="ds">
                  {e.description || (e.transport === "stdio" ? "本地" : "远程")}
                </div>
              </button>
            ))}
          </div>
        </div>

        <div>
          {!entry ? (
            <Empty title="清单为空" hint="点左侧「新增配置」开始" />
          ) : (
            <div className="panel">
              <div className="panel-head">
                编辑条目
                <span className="pill">{entry.transport}</span>
                {entry.auth && <span className="pill">bearer</span>}
                <div className="spacer" />
                <button className="btn small" onClick={removeEntry}>
                  删除此条
                </button>
              </div>
              <div className="panel-body">
                <Field label="名称">
                  <input
                    className="input"
                    value={entry.name}
                    onChange={(e) => patch({ name: e.target.value })}
                  />
                </Field>
                <Field label="描述">
                  <input
                    className="input"
                    value={entry.description ?? ""}
                    onChange={(e) => patch({ description: e.target.value })}
                  />
                </Field>
                <Field label="传输">
                  <select
                    className="select"
                    value={entry.transport}
                    onChange={(e) =>
                      patch({ transport: e.target.value as Entry["transport"] })
                    }
                  >
                    <option value="stdio">stdio（本地进程）</option>
                    <option value="http">http（远程）</option>
                    <option value="sse">sse（远程）</option>
                  </select>
                </Field>
                <Field label="作用域">
                  <select
                    className="select"
                    value={entry.scope ?? "inherit"}
                    onChange={(e) => patch({ scope: e.target.value as Entry["scope"] })}
                  >
                    {SCOPE_OPTIONS.map((s) => (
                      <option key={s.value} value={s.value}>
                        {s.label}
                      </option>
                    ))}
                  </select>
                </Field>

                {entry.transport === "stdio" ? (
                  <>
                    <Field label="命令">
                      <input
                        className="input"
                        value={entry.command ?? ""}
                        onChange={(e) => patch({ command: e.target.value })}
                      />
                    </Field>
                    <Field label="参数">
                      <input
                        className="input"
                        placeholder="逗号分隔，例如 -y, @scope/pkg"
                        value={(entry.args ?? []).join(", ")}
                        onChange={(e) =>
                          patch({
                            args: e.target.value
                              .split(",")
                              .map((x) => x.trim())
                              .filter(Boolean),
                          })
                        }
                      />
                    </Field>
                    <Field label="环境变量">
                      <input
                        className="input"
                        placeholder="KEY=VALUE, KEY2=VALUE2"
                        value={pairToText(entry.env)}
                        onChange={(e) => patch({ env: textToPair(e.target.value) })}
                      />
                    </Field>
                  </>
                ) : (
                  <>
                    <Field label="地址">
                      <input
                        className="input"
                        value={entry.url ?? ""}
                        onChange={(e) => patch({ url: e.target.value })}
                      />
                    </Field>
                    <Field label="Bearer 变量">
                      <input
                        className="input"
                        placeholder="留空表示无需认证，例如 GITHUB_PAT"
                        value={entry.auth?.type === "bearer" ? entry.auth.token_env ?? "" : ""}
                        onChange={(e) =>
                          patch({
                            auth: e.target.value
                              ? { type: "bearer", token_env: e.target.value }
                              : undefined,
                          })
                        }
                      />
                    </Field>
                  </>
                )}

                <Field label="标签">
                  <input
                    className="input"
                    value={(entry.tags ?? []).join(", ")}
                    onChange={(e) =>
                      patch({
                        tags: e.target.value
                          .split(",")
                          .map((x) => x.trim())
                          .filter(Boolean),
                      })
                    }
                  />
                </Field>

                <div style={{ display: "flex", gap: 8, marginTop: 14 }}>
                  <button className="btn primary" disabled={!dirty || saving} onClick={save}>
                    {saving ? "保存中…" : dirty ? "保存到清单" : "已保存"}
                  </button>
                  <span className="muted" style={{ alignSelf: "center" }}>
                    保存会先备份原清单
                  </span>
                </div>
              </div>
            </div>
          )}
        </div>
      </div>
    </>
  );
}

export type { Entry };
