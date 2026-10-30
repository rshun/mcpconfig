import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { ImportAction, ImportChoice, ImportPreview, ImportResult, ImportScan, Scope } from "../types";
import { Banner } from "../components/ui";
import { fileName } from "../library";

type Draft = ImportChoice & { selected: boolean };
const LABEL = { skip: "跳过", add: "新增", rename: "重命名新增", replace: "替换同名项" };

export function ImportPanel({ clientId, catalogPath, scope, projectRoot, onClose, onDirty, onBusy, onImported, embedded = false }: {
  clientId: string; catalogPath: string; scope: Scope; projectRoot: string | null;
  onClose: () => void; onDirty: (dirty: boolean) => void; onBusy: (busy: boolean) => void;
  onImported: (result: ImportResult) => Promise<void>;
  embedded?: boolean;
}) {
  const [scan, setScan] = useState<ImportScan | null>(null);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [acknowledged, setAcknowledged] = useState(false);
  const [busy, setBusy] = useState(false);
  const [committing, setCommitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  const panelRef = useRef<HTMLElement | null>(null);
  const previewRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => { panelRef.current?.scrollIntoView({ block: "start" }); }, []);
  useEffect(() => { if (preview) previewRef.current?.scrollIntoView({ block: "start" }); }, [preview]);

  useEffect(() => {
    let active = true;
    let scanId: string | null = null;
    onDirty(true); setBusy(true);
    api.scanImport(catalogPath, clientId, scope, projectRoot).then((value) => {
      scanId = value.scanId;
      if (!active) { api.cancelImport(scanId).catch(() => {}); return; }
      setScan(value);
      setDrafts(Object.fromEntries(value.items.map((item) => [item.name, {
        selected: false, sourceName: item.name, action: item.conflict ? "skip" : "add",
        targetName: item.name, bindings: Object.fromEntries(item.fields.map((field) => [field.id, ""])),
      } satisfies Draft])));
    }).catch((e) => { if (active) setError(String(e)); }).finally(() => { if (active) setBusy(false); });
    return () => {
      active = false; revision.current++; onDirty(false);
      if (scanId) api.cancelImport(scanId).catch(() => {});
    };
  }, [catalogPath, clientId, scope, projectRoot, onDirty]);

  function patch(name: string, update: Partial<Draft>) {
    revision.current++; setPreview(null); setAcknowledged(false); setError(null);
    setDrafts((prev) => ({ ...prev, [name]: { ...prev[name], ...update } }));
  }
  const chosen = Object.values(drafts).filter((draft) => draft.selected && draft.action !== "skip");
  const ready = chosen.length > 0 && chosen.every((draft) => draft.targetName.trim() &&
    Object.values(draft.bindings).every((name) => /^[A-Za-z_][A-Za-z0-9_]*$/.test(name)));
  async function makePreview() {
    if (!scan || !ready) return;
    const current = ++revision.current; setBusy(true); setError(null); setPreview(null); setAcknowledged(false);
    try {
      const choices = Object.values(drafts).filter((draft) => draft.selected).map(({ selected: _, ...choice }) => choice);
      const value = await api.previewImport(scan.scanId, choices, scope, projectRoot);
      if (current === revision.current) setPreview(value);
    } catch (e) { if (current === revision.current) setError(String(e)); }
    finally { if (current === revision.current) setBusy(false); }
  }
  async function commit() {
    if (!preview || committing) return;
    setCommitting(true); onBusy(true); setError(null);
    try {
      const value = await api.commitImport(preview.previewId, scope, projectRoot);
      onDirty(false); await onImported(value); onClose();
    } catch (e) { setError(String(e)); setPreview(null); panelRef.current?.scrollIntoView({ block: "start" }); }
    finally { setCommitting(false); onBusy(false); }
  }
  const replacing = preview?.changes.some((change) => change.action === "replace") ?? false;
  return <section ref={panelRef} className={`panel import-panel${embedded ? " embedded" : ""}`} aria-label="从 Agent 导入配置库">
    {!embedded && <div className="panel-head"><b>从 Agent 导入配置库</b><div className="spacer" />
      <button className="btn small" disabled={committing} onClick={onClose}>取消导入</button></div>}
    <div className="panel-body">
      <div className="import-route"><span>{clientId}</span><span aria-hidden="true">→</span><b>{fileName(scan?.catalogPath ?? catalogPath)}</b><span className="pill">{scope === "user" ? "全局" : "项目"}</span>{scan && !scan.catalogExists && <small>确认后新建</small>}</div>
      <div className="import-steps"><span className={!preview ? "active" : ""}>1 选择与映射</span><span aria-hidden="true">→</span><span className={preview ? "active" : ""}>2 预览并保存</span></div>
      <details className="import-locations"><summary>来源与目标文件</summary><p>来源：<code>{scan?.sourcePath ?? "扫描中…"}</code></p><p>目标：<code>{scan?.catalogPath ?? catalogPath}</code></p></details>
      <Banner>源 Agent 文件保持不变。原始参数、地址和凭据不会返回前端；隐藏字段需填入环境变量名，不要填写密钥值。
        原环境变量引用会保留。导入不会创建变量或检查变量值，也不会分发到其他 Agent。</Banner>
      {error && <Banner tone="danger">{error}</Banner>}
      <fieldset disabled={busy || committing} className="import-fields">
        {scan?.items.map((item) => {
          const draft = drafts[item.name];
          if (!draft) return null;
          return <article className="import-item" key={item.name}>
            <div className="import-item-head"><label><input type="checkbox" aria-label={`选择导入 ${item.name}`}
              disabled={!!item.blocked} checked={draft.selected} onChange={(e) => patch(item.name, { selected: e.target.checked })} /> <b>{item.name}</b></label>
              <span className={`pill ${item.conflict ? "warn" : "plain"}`}>{item.conflict ? "配置库已有同名项" : item.transport ?? "无法转换"}</span></div>
            {item.blocked ? <p className="muted">不能自动导入：{item.blocked}</p> : draft.selected && <>
              <div className="import-options"><label>同名处理 <select className="select" aria-label={`处理方式 ${item.name}`} value={draft.action}
                onChange={(e) => { const action = e.target.value as ImportAction; patch(item.name, { action, targetName: action === "rename" ? "" : item.name }); }}>
                <option value="skip">跳过</option>{!item.conflict && <option value="add">新增</option>}
                <option value="rename">重命名新增</option>{item.conflict && <option value="replace">替换同名项</option>}
              </select></label>
              {draft.action === "rename" && <label>新名称 <input className="input" aria-label={`目标名称 ${item.name}`} value={draft.targetName} onChange={(e) => patch(item.name, { targetName: e.target.value })} /></label>}</div>
              {draft.action !== "skip" && item.fields.map((field) => <label className="import-mapping" key={field.id}>
                <span>{field.label} <small className="muted">原值已隐藏</small></span>
                <input className="input mono" aria-label={`环境变量 ${item.name} ${field.id}`} autoComplete="off" spellCheck={false}
                  placeholder="例如 MCP_SERVICE_VALUE" value={draft.bindings[field.id]} onChange={(e) => patch(item.name, { bindings: { ...draft.bindings, [field.id]: e.target.value } })} />
              </label>)}
            </>}
          </article>;
        })}
        {scan?.items.length === 0 && <p>来源文件中没有 MCP 条目。</p>}
        <div className="import-controls"><button className="btn" disabled={committing} onClick={onClose}>取消导入</button><div className="spacer" /><button className="btn primary" disabled={!ready || busy || committing} onClick={makePreview}>{busy ? "处理中…" : "预览导入"}</button></div>
      </fieldset>
      {preview && <div ref={previewRef} className="import-preview" aria-label="导入差异预览">
        <h2>将导入 {preview.changes.length} 项</h2>
        <div className="source-path"><span>本次保存到</span><code>{preview.catalogPath}</code></div>
        {preview.changes.map((change) => <details key={change.targetName} open>
          <summary>{LABEL[change.action]} · {change.sourceName} → {change.targetName}</summary>
          {change.before !== null && <><h4>原条目（值已隐藏）</h4><pre>{JSON.stringify(change.before, null, 2)}</pre></>}
          <h4>导入后（仅安全常量和变量引用）</h4><pre>{JSON.stringify(change.after, null, 2)}</pre>
        </details>)}
        {replacing && <Banner tone="warn"><label><input type="checkbox" checked={acknowledged} onChange={(e) => setAcknowledged(e.target.checked)} />
          我确认替换所列同名条目的全部字段，包括原有平台覆盖；写入前保留原文备份。</label></Banner>}
        <button className="btn primary" disabled={committing || busy || (replacing && !acknowledged)} onClick={commit}>{committing ? "导入中…" : "确认导入配置库"}</button>
      </div>}
    </div>
  </section>;
}
