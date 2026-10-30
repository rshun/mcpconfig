import { useState } from "react";
import { api } from "../api";
import { Banner } from "./ui";
import { Icon } from "./Icon";

export function LocationForm({ kind, initial, onUse, onClose, onBusy }: {
  kind: "project" | "library"; initial: string; onUse: (path: string, remember: boolean) => Promise<void>;
  onClose: () => void; onBusy: (busy: boolean) => void;
}) {
  const [path, setPath] = useState(initial);
  const [remember, setRemember] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function use(value: string) {
    setBusy(true); onBusy(true); setError(null);
    try { await onUse(value.trim(), remember); }
    catch (e) { setError(String(e)); }
    finally { setBusy(false); onBusy(false); }
  }
  return <form className="location-form" onSubmit={(event) => { event.preventDefault(); use(path); }}>
    <p className="dialog-intro">{kind === "project" ? "项目范围只影响选定目录，不会使用软件的启动目录。" : "配置库是你统一维护 MCP 的 YAML 或 JSON 文件，与 Agent 原生配置文件分开存放。"}</p>
    {error && <Banner tone="danger">{error}</Banner>}
    <label className="field-label">{kind === "project" ? "项目绝对目录" : "配置库文件绝对路径"}<input autoFocus className="input mono" aria-label={kind === "project" ? "项目目录" : "配置库文件路径"} value={path} disabled={busy} onChange={(event) => setPath(event.target.value)} spellCheck={false} /></label>
    {kind === "project" ? <button className="btn browse-directory" type="button" disabled={busy} onClick={async () => {
      setBusy(true); onBusy(true); setError(null);
      try { const chosen = await api.chooseDirectory(path || null); if (chosen) { setPath(chosen); await onUse(chosen, false); } }
      catch (e) { setError(String(e)); }
      finally { setBusy(false); onBusy(false); }
    }}><Icon name="folder" />选择目录…</button> : <><p className="field-hint">文件尚不存在时，可先使用此位置，首次保存或导入时再创建。已有文件必须是有效的 MCP Hub 配置库。</p>
      <label className="check-line"><input type="checkbox" checked={remember} disabled={busy} onChange={(event) => setRemember(event.target.checked)} />设为启动时使用的配置库</label></>}
    <footer className="editor-footer"><button className="btn" type="button" disabled={busy} onClick={onClose}>取消</button><button className="btn primary" type="submit" disabled={busy || !path.trim()}>{busy ? "正在读取…" : kind === "project" ? "使用此目录" : "使用此配置库"}</button></footer>
  </form>;
}
