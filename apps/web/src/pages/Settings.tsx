import { useState } from "react";
import { api } from "../api";
import type { GuiSettings } from "../types";
import { Banner, Field } from "../components/ui";

export function SettingsPage({ settings, dataDir, onSave, onDirty }: { settings: GuiSettings; dataDir: string;
  onSave: (settings: GuiSettings) => Promise<void>; onDirty: (dirty: boolean) => void }) {
  const [draft, setDraft] = useState(settings);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState("");
  function patch(value: Partial<GuiSettings>) {
    const next = { ...draft, ...value }; setDraft(next); onDirty(JSON.stringify(next) !== JSON.stringify(settings));
  }
  async function save() {
    setSaving(true); setMessage("");
    try { await onSave(draft); onDirty(false); setMessage("设置已保存。默认配置库、范围和写入策略将在下次启动时使用；当前工作范围保持不变。"); }
    catch (e) { setMessage(`保存失败：${String(e)}`); }
    finally { setSaving(false); }
  }
  return <><div className="eyebrow">APPLICATION PREFERENCES</div><h1>全局设置</h1><p className="sub">软件的默认行为，独立于当前工作台中的全局配置和项目配置。</p>
    {message && <Banner tone={message.startsWith("保存失败") ? "danger" : "ok"}>{message}</Banner>}
    <div className="panel settings-panel"><div className="panel-head">启动与存储</div><div className="panel-body"><fieldset disabled={saving} style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }}>
      <Field label="默认配置库文件"><input aria-label="默认配置库文件" className="input" value={draft.catalogPath} onChange={(e) => patch({ catalogPath: e.target.value })} /></Field>
      <p className="muted">使用 YAML 文件的绝对路径。保存设置不会覆盖或移动配置库。</p>
      <Field label="备份目录"><div className="path-picker"><input aria-label="备份目录" className="input grow" value={draft.backupDir} onChange={(e) => patch({ backupDir: e.target.value })} />
        <button className="btn" disabled={saving} onClick={async () => { try { const path = await api.chooseDirectory(draft.backupDir); if (path) patch({ backupDir: path }); } catch (e) { setMessage(String(e)); } }}>浏览…</button></div></Field>
      <p className="muted">保存后新备份写入此处。原目录中的历史备份不会移动，切回原路径即可查看。设置文件自身的备份固定保留在数据目录的 backups 中。</p>
      <Field label="启动默认范围"><select aria-label="启动默认范围" className="select" value={draft.defaultScope} onChange={(e) => patch({ defaultScope: e.target.value as GuiSettings["defaultScope"] })}>
        <option value="user">全局配置 · 当前用户跨项目</option><option value="project">项目配置 · 上次选择的项目</option></select></Field>
      <Field label="默认写入策略"><select aria-label="默认写入策略" className="select" value={draft.defaultMode} onChange={(e) => patch({ defaultMode: e.target.value as GuiSettings["defaultMode"] })}>
        <option value="merge">字段合并 · 保留其他字段</option><option value="replace">整条替换 · 替换选中的同名 MCP</option></select></Field>
      <div className="detail-note">设置文件：<span className="mono">{dataDir}/settings.json</span><br />上次项目：<span className="mono">{draft.lastProjectDir ?? "尚未选择"}</span></div>
      <button className="btn primary" disabled={saving} onClick={save}>{saving ? "保存中…" : "保存全局设置"}</button>
    </fieldset></div></div></>;
}
