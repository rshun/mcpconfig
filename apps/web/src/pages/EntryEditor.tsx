import { useEffect, useState } from "react";
import { Banner } from "../components/ui";
import { Icon } from "../components/Icon";
import { pairMap, pairRows, type PairRow } from "../editor-data";
import type { Entry } from "./Catalog";

function KeyValues({ label, rows, onChange }: { label: string; rows: PairRow[]; onChange: (rows: PairRow[]) => void }) {
  const hasRedacted = rows.some((row) => row.value === "[已隐藏]");
  return <div className="key-values"><div className="field-top"><span className="field-label">{label}</span><button className="text-button" type="button" onClick={() => onChange([...rows, { key: "", value: "" }])}><Icon name="plus" size={14} />添加一项</button></div>
    {rows.map((row, index) => <div className="key-value-row" key={index}><input className="input mono" aria-label={`${label}名称 ${index + 1}`} placeholder="名称" value={row.key} onChange={(event) => onChange(rows.map((item, i) => i === index ? { ...item, key: event.target.value } : item))} />
      <input className="input mono" aria-label={`${label}值 ${index + 1}`} placeholder="值或 ${ENV:VARIABLE}" autoComplete="off" spellCheck={false}
        type={row.value === "[已隐藏]" || /^\$\{(?:ENV:)?[A-Za-z_][A-Za-z0-9_]*\}$/.test(row.value) ? "text" : "password"} value={row.value} onChange={(event) => onChange(rows.map((item, i) => i === index ? { ...item, value: event.target.value } : item))} />
      <button className="btn ghost icon-button" type="button" aria-label={`移除${label} ${index + 1}`} onClick={() => onChange(rows.filter((_, i) => i !== index))}><Icon name="close" size={16} /></button></div>)}
    {hasRedacted && <p className="field-hint">显示为 [已隐藏] 的原凭据仅驻留后端，未修改时保存会自动保留原值。</p>}
    {!rows.length && <p className="field-hint">未设置，按需添加即可。</p>}
  </div>;
}

export function EntryEditor({ entry, entries, onSave, onRemove, onClose, onDirty, onBusy }: {
  entry: Entry | null; entries: Entry[]; onSave: (entry: Entry) => Promise<void>; onRemove: (() => Promise<void>) | null;
  onClose: () => void; onDirty: (dirty: boolean) => void; onBusy: (busy: boolean) => void;
}) {
  const initial: Entry = entry ?? { name: "", transport: "stdio", command: "npx", args: [], scope: "inherit" };
  const [draft, setDraft] = useState<Entry>(initial);
  const [env, setEnv] = useState(pairRows(initial.env));
  const [headers, setHeaders] = useState(pairRows(initial.headers));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dirty = JSON.stringify(draft) !== JSON.stringify(initial) || JSON.stringify(env) !== JSON.stringify(pairRows(initial.env)) || JSON.stringify(headers) !== JSON.stringify(pairRows(initial.headers));
  useEffect(() => { onDirty(dirty); }, [dirty, onDirty]);
  useEffect(() => () => onDirty(false), [onDirty]);
  function patch(value: Partial<Entry>) { setDraft((prev) => ({ ...prev, ...value })); setError(null); }
  async function save() {
    setError(null);
    try {
      if (!draft.name.trim() || draft.name.trim() !== draft.name || /[\r\n\t]/.test(draft.name)) throw new Error("请填写名称，且不要在名称两端留空格");
      if (entries.some((item) => item.name === draft.name && item.name !== entry?.name)) throw new Error("配置库已有同名 MCP，请使用其他名称");
      if (draft.transport === "stdio" ? !draft.command?.trim() : !draft.url?.trim()) throw new Error(draft.transport === "stdio" ? "请填写启动命令" : "请填写远程地址或地址变量引用");
      if (draft.auth?.type === "bearer" && draft.auth.token_env && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(draft.auth.token_env)) throw new Error("Bearer 字段填写环境变量名，不要填写密钥值");
      const next = { ...draft, env: pairMap(env), headers: pairMap(headers, true) };
      if (!entry || entry.transport !== draft.transport) {
        if (draft.transport === "stdio") { next.url = undefined; next.auth = undefined; next.headers = undefined; }
        else { next.command = undefined; next.args = []; next.cwd = undefined; }
      }
      setSaving(true); onBusy(true); await onSave(next); onDirty(false);
    } catch (e) { setError(String(e instanceof Error ? e.message : e)); }
    finally { setSaving(false); onBusy(false); }
  }
  return <form className="entry-form" noValidate onSubmit={(event) => { event.preventDefault(); save(); }}>
    <p className="dialog-intro">先保存到配置库，再选择需要应用的 Agent。保存不会分发配置。</p>
    {error && <Banner tone="danger">{error}</Banner>}
    <fieldset disabled={saving} className="editor-fields"><div className="form-two-col">
      <label className="field-label">MCP 名称<span className="required-mark">必填</span><input autoFocus className="input" aria-label="MCP 名称" placeholder="例如 filesystem" value={draft.name} onChange={(event) => patch({ name: event.target.value })} /></label>
      <label className="field-label">连接方式<select className="select" aria-label="连接方式" value={draft.transport} onChange={(event) => patch({ transport: event.target.value as Entry["transport"] })}><option value="stdio">本地命令 · stdio</option><option value="http">远程服务 · HTTP</option><option value="sse">远程服务 · SSE</option></select></label></div>
      <label className="field-label">描述<span className="optional-mark">可选</span><input className="input" aria-label="MCP 描述" placeholder="用一句话说明它的用途" value={draft.description ?? ""} onChange={(event) => patch({ description: event.target.value })} /></label>
      {draft.transport === "stdio" ? <>
        <label className="field-label">启动命令<span className="required-mark">必填</span><input className="input mono" aria-label="启动命令" placeholder="例如 npx、python 或程序绝对路径" value={draft.command ?? ""} onChange={(event) => patch({ command: event.target.value })} /></label>
        <div className="field-top"><span className="field-label">启动参数</span><button className="text-button" type="button" onClick={() => patch({ args: [...(draft.args ?? []), ""] })}><Icon name="plus" size={14} />添加参数</button></div>
        <p className="field-hint">每项一个输入框，逗号和空格会原样保留，空值表示一个空参数。</p>
        {(draft.args ?? []).map((arg, index) => <div className="argument-row" key={index}><span className="argument-index">{index + 1}</span><input className="input mono" aria-label={`启动参数 ${index + 1}`} value={arg} onChange={(event) => patch({ args: draft.args!.map((value, i) => i === index ? event.target.value : value) })} />
          <button className="btn ghost icon-button" type="button" aria-label={`移除启动参数 ${index + 1}`} onClick={() => patch({ args: draft.args!.filter((_, i) => i !== index) })}><Icon name="close" size={16} /></button></div>)}
        <KeyValues label="环境变量" rows={env} onChange={setEnv} />
      </> : <>
        <label className="field-label">服务地址<span className="required-mark">必填</span><input className="input mono" inputMode="url" aria-label="服务地址" placeholder="https://example.invalid/mcp 或 ${ENV:MCP_URL}" value={draft.url ?? ""} onChange={(event) => patch({ url: event.target.value })} /></label>
        {draft.auth && draft.auth.type !== "bearer" ? (
          <p className="field-hint">已配置自定义请求头认证（{draft.auth.header}），保存时保留原设置。</p>
        ) : <>
          <label className="field-label">Bearer 凭据变量<span className="optional-mark">无需认证可留空</span><input className="input mono" aria-label="Bearer 凭据变量" placeholder="例如 SERVICE_API_KEY，只填变量名" autoComplete="off" value={draft.auth?.type === "bearer" ? draft.auth.token_env ?? "" : ""} onChange={(event) => patch({ auth: event.target.value ? { type: "bearer", token_env: event.target.value } : undefined })} /></label>
          {draft.auth?.type === "bearer" && draft.auth.token && <p className="field-hint">原配置含直接凭据，未改动时保留原值；填写变量名会将它替换为变量引用。</p>}
        </>}
      </>}
      <details className="advanced-editor"><summary>高级设置<span>{draft.scope && draft.scope !== "inherit" ? ` · ${draft.scope === "project" ? "仅项目" : draft.scope === "user" ? "仅全局" : "仅本地"}` : " · 按需展开"}</span></summary><div className="advanced-editor-body">
        <label className="field-label">适用范围<select className="select" aria-label="适用范围" value={draft.scope ?? "inherit"} onChange={(event) => patch({ scope: event.target.value as Entry["scope"] })}><option value="inherit">跟随工作范围（推荐）</option><option value="user">仅全局配置</option><option value="project">仅项目配置</option><option value="local">仅本地配置（项目私有，不入库）</option></select></label>
        {draft.transport === "stdio" && <label className="field-label">工作目录<input className="input mono" aria-label="启动工作目录" value={draft.cwd ?? ""} placeholder="可使用 ${PROJECT_ROOT}" onChange={(event) => patch({ cwd: event.target.value || undefined })} /></label>}
        {draft.transport !== "stdio" && <><KeyValues label="环境变量" rows={env} onChange={setEnv} /><KeyValues label="请求头" rows={headers} onChange={setHeaders} /></>}
        <p className="field-hint">已有的平台覆盖、标签、启用状态和其他高级字段会保留。变量值和连接状态不会在此验证。</p>
      </div></details>
      {entry && entry.transport !== draft.transport && <Banner tone="warn">保存时会清除另一类连接方式的字段，请核对命令或地址。</Banner>}
    </fieldset>
    <footer className="editor-footer">{onRemove && <button className="btn danger ghost" type="button" disabled={saving} onClick={async () => {
      if (!window.confirm(`从配置库移除 ${entry?.name}？已分发到 Agent 的配置保持不变，原配置库会先备份。`)) return;
      setSaving(true); onBusy(true); try { await onRemove(); onDirty(false); } catch (e) { setError(String(e)); } finally { setSaving(false); onBusy(false); }
    }}>移除此 MCP</button>}<div className="spacer" /><button className="btn" type="button" disabled={saving} onClick={onClose}>取消</button><button className="btn primary" disabled={saving || (!dirty && !!entry)} type="submit">{saving ? "正在保存…" : "保存到 MCP 库"}</button></footer>
  </form>;
}
