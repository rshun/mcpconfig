import { useEffect, useState } from "react";
import { Banner } from "../components/ui";
import { Icon } from "../components/Icon";
import { pairMap, pairRows, type PairRow } from "../editor-data";
import { parseMcpSnippet } from "../snippet";
import type { Entry } from "./Catalog";

function KeyValues({
  label,
  rows,
  onChange,
}: {
  label: string;
  rows: PairRow[];
  onChange: (rows: PairRow[]) => void;
}) {
  const [revealed, setRevealed] = useState<Record<number, boolean>>({});
  const hasRedacted = rows.some((row) => row.value === "[已隐藏]");
  return (
    <div className="key-values">
      <div className="field-top">
        <span className="field-label">{label}</span>
        <button
          className="text-button"
          type="button"
          onClick={() => onChange([...rows, { key: "", value: "" }])}
        >
          <Icon name="plus" size={14} />
          添加一项
        </button>
      </div>
      {rows.map((row, index) => {
        const isPlainRef =
          row.value === "[已隐藏]" ||
          /^\$\{(?:ENV:|env:)?[A-Za-z_][A-Za-z0-9_]*\}$/.test(row.value);
        const showPlain = isPlainRef || !!revealed[index];
        return (
          <div className="key-value-row" key={index}>
            <input
              className="input mono"
              aria-label={`${label}名称 ${index + 1}`}
              placeholder="名称"
              value={row.key}
              onChange={(event) =>
                onChange(
                  rows.map((item, i) =>
                    i === index ? { ...item, key: event.target.value } : item,
                  ),
                )
              }
            />
            <input
              className="input mono"
              aria-label={`${label}值 ${index + 1}`}
              placeholder="值或 ${ENV:VARIABLE}"
              autoComplete="off"
              spellCheck={false}
              type={showPlain ? "text" : "password"}
              value={row.value}
              onChange={(event) =>
                onChange(
                  rows.map((item, i) =>
                    i === index ? { ...item, value: event.target.value } : item,
                  ),
                )
              }
            />
            {!isPlainRef && (
              <button
                className="btn ghost icon-button small"
                type="button"
                aria-label={`${showPlain ? "隐藏" : "显示"}${label}值 ${index + 1}`}
                title={showPlain ? "隐藏明文" : "查看输入内容"}
                onClick={() =>
                  setRevealed((prev) => ({ ...prev, [index]: !prev[index] }))
                }
              >
                <Icon name={showPlain ? "eyeOff" : "eye"} size={15} />
              </button>
            )}
            <button
              className="btn ghost icon-button"
              type="button"
              aria-label={`移除${label} ${index + 1}`}
              onClick={() => {
                setRevealed((prev) => {
                  const next: Record<number, boolean> = {};
                  Object.entries(prev).forEach(([k, v]) => {
                    const n = Number(k);
                    if (n < index) next[n] = v;
                    else if (n > index) next[n - 1] = v;
                  });
                  return next;
                });
                onChange(rows.filter((_, i) => i !== index));
              }}
            >
              <Icon name="close" size={16} />
            </button>
          </div>
        );
      })}
      {hasRedacted && (
        <p className="field-hint">
          显示为 [已隐藏] 的原凭据仅驻留后端，未修改时保存会自动保留原值。
        </p>
      )}
      {!rows.length && <p className="field-hint">未设置，按需添加即可。</p>}
    </div>
  );
}

export function EntryEditor({
  entry,
  entries,
  onSave,
  onRemove,
  onClose,
  onDirty,
  onBusy,
}: {
  entry: Entry | null;
  entries: Entry[];
  onSave: (entry: Entry) => Promise<void>;
  onRemove: (() => Promise<void>) | null;
  onClose: () => void;
  onDirty: (dirty: boolean) => void;
  onBusy: (busy: boolean) => void;
}) {
  const initial: Entry = entry ?? {
    name: "",
    transport: "stdio",
    command: "npx",
    args: [],
    scope: "inherit",
  };
  const [draft, setDraft] = useState<Entry>(initial);
  const [env, setEnv] = useState(pairRows(initial.env));
  const [headers, setHeaders] = useState(pairRows(initial.headers));
  const [tagsInput, setTagsInput] = useState((initial.tags ?? []).join(", "));
  const [tagsEdited, setTagsEdited] = useState(false);
  const [snippetOpen, setSnippetOpen] = useState(!entry);
  const [snippetText, setSnippetText] = useState("");
  const [snippetHint, setSnippetHint] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const dirty =
    JSON.stringify(draft) !== JSON.stringify(initial) ||
    JSON.stringify(env) !== JSON.stringify(pairRows(initial.env)) ||
    JSON.stringify(headers) !== JSON.stringify(pairRows(initial.headers)) ||
    (tagsEdited && tagsInput !== (initial.tags ?? []).join(", "));

  useEffect(() => {
    onDirty(dirty);
  }, [dirty, onDirty]);
  useEffect(() => () => onDirty(false), [onDirty]);

  function patch(value: Partial<Entry>) {
    setDraft((prev) => ({ ...prev, ...value }));
    setError(null);
  }

  function applySnippet(raw: string) {
    setError(null);
    setSnippetHint(null);
    try {
      const res = parseMcpSnippet(raw);
      const p = res.entry;
      setDraft((prev) => ({
        ...prev,
        name: prev.name.trim() && entry ? prev.name : p.name || prev.name,
        transport: p.transport ?? prev.transport,
        command: p.transport === "stdio" ? (p.command ?? prev.command) : undefined,
        args: p.transport === "stdio" ? (p.args ?? []) : [],
        cwd: p.cwd ?? prev.cwd,
        url: p.transport !== "stdio" ? (p.url ?? prev.url) : undefined,
        auth: p.auth ?? (p.transport !== "stdio" ? prev.auth : undefined),
        description: p.description ?? prev.description,
        scope: p.scope ?? prev.scope,
        ...(p.enabled !== undefined ? { enabled: p.enabled } : {}),
        ...(p.timeout_seconds !== undefined ? { timeout_seconds: p.timeout_seconds } : {}),
      }));
      if (p.env) setEnv(pairRows(p.env));
      if (p.headers) setHeaders(pairRows(p.headers));
      setSnippetText("");
      setSnippetOpen(false);
      const multiNote = res.totalFound > 1 ? `（检测到 ${res.totalFound} 项，已填入首项）` : "";
      setSnippetHint(`已从 ${res.sourceKind.toUpperCase()} 自动识别并填入配置${multiNote}，请核对后保存。`);
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    }
  }

  const authKind: "bearer" | "header" =
    draft.auth && draft.auth.type !== "bearer" ? "header" : "bearer";

  async function save() {
    setError(null);
    try {
      if (!draft.name.trim() || draft.name.trim() !== draft.name || /[\r\n\t]/.test(draft.name)) {
        throw new Error("请填写名称，且不要在名称两端留空格");
      }
      if (entries.some((item) => item.name === draft.name && item.name !== entry?.name)) {
        throw new Error("配置库已有同名 MCP，请使用其他名称");
      }
      if (draft.transport === "stdio" ? !draft.command?.trim() : !draft.url?.trim()) {
        throw new Error(draft.transport === "stdio" ? "请填写启动命令" : "请填写远程地址或地址变量引用");
      }
      if (draft.auth?.type === "bearer" && draft.auth.token_env && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(draft.auth.token_env)) {
        throw new Error("Bearer 字段填写环境变量名，不要填写密钥值");
      }
      if (draft.auth && draft.auth.type !== "bearer") {
        if (!draft.auth.header.trim()) {
          throw new Error("请填写自定义认证请求头名称（例如 X-API-Key）");
        }
        const envRef = draft.auth.token_env || draft.auth.value_env || "";
        if (!envRef.trim() && !draft.auth.token && !draft.auth.value) {
          throw new Error("请填写自定义请求头对应的环境变量名");
        }
        if (envRef && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(envRef)) {
          throw new Error("认证凭据变量需填写合法环境变量名（例如 API_KEY）");
        }
      }

      const parsedTags = tagsEdited
        ? tagsInput
            .split(",")
            .map((t) => t.trim())
            .filter(Boolean)
        : draft.tags;

      const next: Entry = {
        ...draft,
        env: pairMap(env),
        headers: pairMap(headers, true),
        ...(tagsEdited ? { tags: parsedTags?.length ? parsedTags : undefined } : {}),
      };
      if (!entry || entry.transport !== draft.transport) {
        if (draft.transport === "stdio") {
          next.url = undefined;
          next.auth = undefined;
          next.headers = undefined;
        } else {
          next.command = undefined;
          next.args = [];
          next.cwd = undefined;
        }
      }
      setSaving(true);
      onBusy(true);
      await onSave(next);
      onDirty(false);
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setSaving(false);
      onBusy(false);
    }
  }

  const isEnabled = draft.enabled !== false;

  return (
    <form
      className="entry-form"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        save();
      }}
    >
      <div className="editor-top-bar">
        <p className="dialog-intro">先保存到配置库，再按需分发到目标 Agent。</p>
        <button
          type="button"
          className={`btn small ${snippetOpen ? "ghost" : ""}`}
          onClick={() => setSnippetOpen((prev) => !prev)}
        >
          <Icon name="code" size={15} />
          {snippetOpen ? "收起粘贴框" : "粘贴 JSON / 命令"}
        </button>
      </div>

      {snippetOpen && (
        <div className="snippet-import-box">
          <div className="snippet-box-head">
            <span>
              <b>一键识别配置片段</b>
              <small>支持粘贴 mcpServers JSON、OpenCode JSONC、claude mcp add 或 npx 启动命令</small>
            </span>
            {typeof navigator !== "undefined" && !!navigator.clipboard && (
              <button
                type="button"
                className="text-button"
                onClick={async () => {
                  try {
                    const clip = await navigator.clipboard.readText();
                    if (clip.trim()) {
                      setSnippetText(clip);
                      applySnippet(clip);
                    }
                  } catch {
                    setError("无法直接读取剪贴板，请在下方输入框按 Ctrl+V 粘贴");
                  }
                }}
              >
                读取剪贴板并填入
              </button>
            )}
          </div>
          <textarea
            className="textarea mono snippet-textarea"
            aria-label="配置片段粘贴框"
            rows={3}
            placeholder={'例如：npx -y @playwright/mcp@latest 或 {"mcpServers": {"memory": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-memory"]}}}'}
            value={snippetText}
            onChange={(event) => setSnippetText(event.target.value)}
          />
          <div className="snippet-box-actions">
            <span className="field-hint">自动提取命令、参数、环境变量并转为统一引用语法</span>
            <button
              type="button"
              className="btn primary small"
              disabled={!snippetText.trim()}
              onClick={() => applySnippet(snippetText)}
            >
              解析并填入表单
            </button>
          </div>
        </div>
      )}

      {snippetHint && <Banner tone="ok">{snippetHint}</Banner>}
      {error && <Banner tone="danger">{error}</Banner>}

      <fieldset disabled={saving} className="editor-fields">
        <div className="enable-switch-row">
          <div>
            <b>启用此 MCP</b>
            <small>
              {isEnabled
                ? "正常参与当前范围的状态对比与分发选择"
                : "已停用：保留在配置库中，暂不参与新的分发计划"}
            </small>
          </div>
          <button
            type="button"
            role="switch"
            aria-label="启用此 MCP"
            aria-checked={isEnabled}
            className={`toggle-switch${isEnabled ? " on" : ""}`}
            onClick={() => patch({ enabled: isEnabled ? false : true })}
          >
            <span className="toggle-thumb" />
          </button>
        </div>

        <div className="form-two-col">
          <label className="field-label">
            MCP 名称
            <span className="required-mark">必填</span>
            <input
              autoFocus
              className="input"
              aria-label="MCP 名称"
              placeholder="例如 filesystem"
              value={draft.name}
              onChange={(event) => patch({ name: event.target.value })}
            />
          </label>
          <label className="field-label">
            连接方式
            <select
              className="select"
              aria-label="连接方式"
              value={draft.transport}
              onChange={(event) =>
                patch({ transport: event.target.value as Entry["transport"] })
              }
            >
              <option value="stdio">本地命令 · stdio</option>
              <option value="http">远程服务 · HTTP</option>
              <option value="sse">远程服务 · SSE</option>
            </select>
          </label>
        </div>

        <label className="field-label">
          描述
          <span className="optional-mark">可选</span>
          <input
            className="input"
            aria-label="MCP 描述"
            placeholder="用一句话说明它的用途"
            value={draft.description ?? ""}
            onChange={(event) => patch({ description: event.target.value })}
          />
        </label>

        {draft.transport === "stdio" ? (
          <>
            <label className="field-label">
              启动命令
              <span className="required-mark">必填</span>
              <input
                className="input mono"
                aria-label="启动命令"
                placeholder="例如 npx、uvx、python 或程序绝对路径"
                value={draft.command ?? ""}
                onChange={(event) => patch({ command: event.target.value })}
              />
            </label>
            <div className="field-top">
              <span className="field-label">启动参数</span>
              <button
                className="text-button"
                type="button"
                onClick={() => patch({ args: [...(draft.args ?? []), ""] })}
              >
                <Icon name="plus" size={14} />
                添加参数
              </button>
            </div>
            <p className="field-hint">
              每项独立保存，逗号与空格原样保留（无需手动加引号）。
            </p>
            {(draft.args ?? []).map((arg, index) => (
              <div className="argument-row" key={index}>
                <span className="argument-index">{index + 1}</span>
                <input
                  className="input mono"
                  aria-label={`启动参数 ${index + 1}`}
                  value={arg}
                  onChange={(event) =>
                    patch({
                      args: draft.args!.map((value, i) =>
                        i === index ? event.target.value : value,
                      ),
                    })
                  }
                />
                <button
                  className="btn ghost icon-button"
                  type="button"
                  aria-label={`移除启动参数 ${index + 1}`}
                  onClick={() =>
                    patch({ args: draft.args!.filter((_, i) => i !== index) })
                  }
                >
                  <Icon name="close" size={16} />
                </button>
              </div>
            ))}
            <KeyValues label="环境变量" rows={env} onChange={setEnv} />
          </>
        ) : (
          <>
            <label className="field-label">
              服务地址
              <span className="required-mark">必填</span>
              <input
                className="input mono"
                inputMode="url"
                aria-label="服务地址"
                placeholder="https://example.invalid/mcp 或 ${ENV:MCP_URL}"
                value={draft.url ?? ""}
                onChange={(event) => patch({ url: event.target.value })}
              />
            </label>
            <div className="form-two-col">
              <label className="field-label">
                认证方式
                <select
                  className="select"
                  aria-label="认证方式"
                  value={authKind}
                  onChange={(event) => {
                    const nextKind = event.target.value as "bearer" | "header";
                    if (nextKind === "bearer") {
                      const prevEnv =
                        draft.auth && draft.auth.type !== "bearer"
                          ? draft.auth.token_env || draft.auth.value_env
                          : draft.auth?.token_env;
                      patch({
                        auth: prevEnv ? { type: "bearer", token_env: prevEnv } : undefined,
                      });
                    } else {
                      const prevEnv =
                        draft.auth?.type === "bearer" ? draft.auth.token_env : undefined;
                      patch({
                        auth: {
                          type: "header",
                          header: "X-API-Key",
                          ...(prevEnv ? { token_env: prevEnv } : {}),
                        },
                      });
                    }
                  }}
                >
                  <option value="bearer">Bearer Token（标准 Authorization）</option>
                  <option value="header">自定义请求头（Header / API Key）</option>
                </select>
              </label>
              {authKind === "bearer" ? (
                <label className="field-label">
                  Bearer 凭据变量
                  <span className="optional-mark">无需认证可留空</span>
                  <input
                    className="input mono"
                    aria-label="Bearer 凭据变量"
                    placeholder="例如 GITHUB_PAT，只填变量名"
                    autoComplete="off"
                    value={draft.auth?.type === "bearer" ? draft.auth.token_env ?? "" : ""}
                    onChange={(event) =>
                      patch({
                        auth: event.target.value
                          ? { type: "bearer", token_env: event.target.value }
                          : undefined,
                      })
                    }
                  />
                </label>
              ) : (
                <label className="field-label">
                  凭据环境变量名
                  <span className="required-mark">必填</span>
                  <input
                    className="input mono"
                    aria-label="凭据环境变量名"
                    placeholder="例如 CONTEXT7_API_KEY"
                    autoComplete="off"
                    value={
                      draft.auth && draft.auth.type !== "bearer"
                        ? draft.auth.token_env ?? draft.auth.value_env ?? ""
                        : ""
                    }
                    onChange={(event) => {
                      if (!draft.auth || draft.auth.type === "bearer") return;
                      patch({
                        auth: {
                          type: "header",
                          header: draft.auth.header,
                          ...(draft.auth.prefix ? { prefix: draft.auth.prefix } : {}),
                          token_env: event.target.value,
                        },
                      });
                    }}
                  />
                </label>
              )}
            </div>
            {authKind === "header" && draft.auth && draft.auth.type !== "bearer" && (
              <div className="form-two-col" style={{ marginTop: 14 }}>
                <label className="field-label">
                  请求头名称
                  <span className="required-mark">必填</span>
                  <input
                    className="input mono"
                    aria-label="认证请求头名称"
                    placeholder="例如 X-API-Key"
                    value={draft.auth.header}
                    onChange={(event) => {
                      if (!draft.auth || draft.auth.type === "bearer") return;
                      patch({
                        auth: { ...draft.auth, header: event.target.value },
                      });
                    }}
                  />
                </label>
                <label className="field-label">
                  值前缀
                  <span className="optional-mark">可选</span>
                  <input
                    className="input mono"
                    aria-label="认证值前缀"
                    placeholder="例如 ApiKey 或留空"
                    value={draft.auth.prefix ?? ""}
                    onChange={(event) => {
                      if (!draft.auth || draft.auth.type === "bearer") return;
                      const nextPrefix = event.target.value;
                      const copy = { ...draft.auth };
                      if (nextPrefix) copy.prefix = nextPrefix;
                      else delete copy.prefix;
                      patch({ auth: copy });
                    }}
                  />
                </label>
              </div>
            )}
            {draft.auth?.type === "bearer" && draft.auth.token && (
              <p className="field-hint">
                原配置含直接凭据，未改动时保留原值；填写变量名会将它替换为安全的环境变量引用。
              </p>
            )}
          </>
        )}

        <details className="advanced-editor">
          <summary>
            高级设置
            <span>
              {draft.scope && draft.scope !== "inherit"
                ? ` · ${draft.scope === "project" ? "仅项目" : draft.scope === "user" ? "仅全局" : "仅本地"}`
                : ""}
              {draft.timeout_seconds ? ` · 超时 ${draft.timeout_seconds}s` : ""}
              {(!draft.scope || draft.scope === "inherit") && !draft.timeout_seconds
                ? " · 范围、超时、标签与请求头"
                : ""}
            </span>
          </summary>
          <div className="advanced-editor-body">
            <div className="form-two-col">
              <label className="field-label">
                适用范围
                <select
                  className="select"
                  aria-label="适用范围"
                  value={draft.scope ?? "inherit"}
                  onChange={(event) =>
                    patch({ scope: event.target.value as Entry["scope"] })
                  }
                >
                  <option value="inherit">跟随工作范围（推荐）</option>
                  <option value="user">仅全局配置</option>
                  <option value="project">仅项目配置</option>
                  <option value="local">仅本地配置（项目私有，不入库）</option>
                </select>
              </label>
              <label className="field-label">
                启动/连接超时（秒）
                <span className="optional-mark">可选</span>
                <input
                  className="input mono"
                  type="number"
                  min={1}
                  aria-label="超时秒数"
                  placeholder="默认由客户端决定，例如 30"
                  value={draft.timeout_seconds ?? ""}
                  onChange={(event) => {
                    const raw = event.target.value.trim();
                    const num = raw ? Number(raw) : undefined;
                    patch({
                      timeout_seconds:
                        num !== undefined && Number.isFinite(num) && num > 0
                          ? Math.floor(num)
                          : undefined,
                    });
                  }}
                />
              </label>
            </div>
            {draft.transport === "stdio" && (
              <label className="field-label">
                工作目录
                <input
                  className="input mono"
                  aria-label="启动工作目录"
                  value={draft.cwd ?? ""}
                  placeholder="可使用 ${PROJECT_ROOT}"
                  onChange={(event) =>
                    patch({ cwd: event.target.value || undefined })
                  }
                />
              </label>
            )}
            <label className="field-label">
              分类标签
              <span className="optional-mark">逗号分隔</span>
              <input
                className="input"
                aria-label="分类标签"
                placeholder="例如 搜索, 数据库, 前端"
                value={tagsInput}
                onChange={(event) => {
                  setTagsInput(event.target.value);
                  setTagsEdited(true);
                  setError(null);
                }}
              />
            </label>
            {draft.transport !== "stdio" && (
              <>
                <KeyValues label="环境变量" rows={env} onChange={setEnv} />
                <KeyValues label="请求头" rows={headers} onChange={setHeaders} />
              </>
            )}
            <p className="field-hint">
              已有的跨平台覆盖（overrides）与客户端扩展字段（extensions）在保存时自动无损保留。
            </p>
          </div>
        </details>
        {entry && entry.transport !== draft.transport && (
          <Banner tone="warn">
            保存时会清除另一类连接方式的字段，请核对命令或地址。
          </Banner>
        )}
      </fieldset>

      <footer className="editor-footer">
        {onRemove && !confirmRemove && (
          <button
            className="btn danger ghost"
            type="button"
            disabled={saving}
            onClick={() => setConfirmRemove(true)}
          >
            <Icon name="trash" size={15} />
            移除此 MCP
          </button>
        )}
        {onRemove && confirmRemove && (
          <div className="inline-confirm" role="group" aria-label="确认移除此 MCP">
            <span className="inline-confirm-text">从配置库移除？（不影响已写入 Agent 的文件）</span>
            <button
              className="btn danger small"
              type="button"
              disabled={saving}
              onClick={async () => {
                setSaving(true);
                onBusy(true);
                try {
                  await onRemove();
                  onDirty(false);
                } catch (e) {
                  setError(String(e));
                  setConfirmRemove(false);
                } finally {
                  setSaving(false);
                  onBusy(false);
                }
              }}
            >
              确认移除
            </button>
            <button
              className="btn ghost small"
              type="button"
              disabled={saving}
              onClick={() => setConfirmRemove(false)}
            >
              取消
            </button>
          </div>
        )}
        <div className="spacer" />
        <button className="btn" type="button" disabled={saving} onClick={onClose}>
          取消
        </button>
        <button
          className="btn primary"
          disabled={saving || (!dirty && !!entry)}
          type="submit"
        >
          {saving ? "正在保存…" : "保存到 MCP 库"}
        </button>
      </footer>
    </form>
  );
}
