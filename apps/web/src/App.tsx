import { useCallback, useEffect, useRef, useState } from "react";
import { api, isMock } from "./api";
import { selectionFromCells } from "./selection";
import type { AppInfoDto, BackupDto, CatalogDto, ClientDto, GuiSettings, MatrixDto, MergeMode, PlanDto, PlanInput, ReceiptDto, Scope } from "./types";
import { MatrixPage } from "./pages/Matrix";
import { CatalogPage, type Entry } from "./pages/Catalog";
import { DiffPage } from "./pages/Diff";
import { BackupsPage, DetectPage } from "./pages/Panels";
import { Workbench } from "./pages/Workbench";
import { SettingsPage } from "./pages/Settings";
import { Banner } from "./components/ui";

type Page = "workbench" | "matrix" | "catalog" | "diff" | "detect" | "backups" | "settings";
const NAV: { id: Page; label: string; icon: string }[] = [
  { id: "workbench", label: "配置工作台", icon: "▦" }, { id: "detect", label: "Agent", icon: "◎" },
  { id: "backups", label: "变更记录", icon: "↶" }, { id: "settings", label: "全局设置", icon: "⚙" },
];
const initialPage = new URLSearchParams(window.location.search).get("page") as Page;
export default function App() {
  const [page, setPage] = useState<Page>(["workbench", "matrix", "catalog", "diff", "detect", "backups", "settings"].includes(initialPage) ? initialPage : "workbench");
  const [settings, setSettings] = useState<GuiSettings | null>(null);
  const [settingsValid, setSettingsValid] = useState(false);
  const [catalogPath, setCatalogPath] = useState("");
  const [pathInput, setPathInput] = useState("");
  const [scope, setScope] = useState<Scope>("user");
  const [project, setProject] = useState<string | null>(null);
  const [projectInput, setProjectInput] = useState("");
  const [mode, setMode] = useState<MergeMode>("merge");
  const [info, setInfo] = useState<AppInfoDto | null>(null);
  const [matrix, setMatrix] = useState<MatrixDto | null>(null);
  const [clients, setClients] = useState<ClientDto[] | null>(null);
  const [catalog, setCatalog] = useState<CatalogDto | null>(null);
  const [entries, setEntries] = useState<Entry[]>([]);
  const [backups, setBackups] = useState<BackupDto[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [plan, setPlan] = useState<PlanDto | null>(null);
  const [planRequest, setPlanRequest] = useState<PlanInput | null>(null);
  const [receipt, setReceipt] = useState<ReceiptDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [applying, setApplying] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const planRevision = useRef(0);
  const readRevision = useRef(0);
  const projectRoot = scope === "user" ? null : project;
  const contextReady = settingsValid && (scope === "user" || !!project);
  const invalidatePlan = useCallback(() => {
    planRevision.current++; setPlan(null); setPlanRequest(null);
  }, []);
  function canLeave() { return !dirty || window.confirm("有未保存的编辑，确认放弃这些编辑？"); }
  function navigate(next: Page) { if (applying || !canLeave()) return; setDirty(false); setPage(next); }

  const refresh = useCallback(async (path: string, sc: Scope, root: string | null) => {
    invalidatePlan(); const revision = ++readRevision.current;
    setLoading(true); setError(null); setSelected(new Set()); setMatrix(null); setClients(null);
    const effective = sc === "user" ? null : root;
    if (sc !== "user" && !effective) { setLoading(false); return; }
    const [m, c] = await Promise.allSettled([api.matrix(path, sc, effective), api.detect(sc, effective)]);
    if (revision !== readRevision.current) return;
    setMatrix(m.status === "fulfilled" ? m.value : null);
    setClients(c.status === "fulfilled" ? c.value : null);
    const failures = [m, c].flatMap((r) => r.status === "rejected" ? [String(r.reason)] : []);
    if (failures.length) setError(failures.join("；"));
    setLoading(false);
  }, [invalidatePlan]);
  const catalogRevision = useRef(0);
  const loadCatalog = useCallback(async (path: string) => {
    const revision = ++catalogRevision.current;
    try { const c = await api.loadCatalog(path); if (revision === catalogRevision.current) { setCatalog(c); setEntries((c.catalog?.entries ?? []) as Entry[]); } }
    catch { if (revision === catalogRevision.current) { setCatalog(null); setEntries([]); } }
  }, []);
  const loadBackups = useCallback(async () => {
    try { setBackups(await api.backups()); } catch (e) { setBackups([]); setError(String(e)); }
  }, []);
  useEffect(() => {
    let active = true;
    api.appInfo().then((value) => { if (active) setInfo(value); }).catch(() => {});
    (async () => {
      try {
        const value = await api.loadSettings(); if (!active) return;
        setSettings(value); setSettingsValid(true); setCatalogPath(value.catalogPath); setPathInput(value.catalogPath);
        setScope(value.defaultScope); setMode(value.defaultMode); setProject(value.lastProjectDir); setProjectInput(value.lastProjectDir ?? "");
        await Promise.all([refresh(value.catalogPath, value.defaultScope, value.lastProjectDir), loadCatalog(value.catalogPath), loadBackups()]);
      } catch (e) {
        if (!active) return;
        setError(String(e)); setLoading(false); setSettingsValid(false); setPage("settings");
        try { const defaults = await api.defaultSettings(); if (active) setSettings(defaults); } catch { /* 保留原始错误 */ }
      }
    })();
    return () => { active = false; readRevision.current++; catalogRevision.current++; };
  }, [refresh, loadCatalog, loadBackups]);
  useEffect(() => {
    const warn = (event: BeforeUnloadEvent) => { if (dirty || applying) { event.preventDefault(); event.returnValue = ""; } };
    window.addEventListener("beforeunload", warn); return () => window.removeEventListener("beforeunload", warn);
  }, [dirty, applying]);
  useEffect(() => { if (page === "backups" && settingsValid) loadBackups(); }, [page, settingsValid, loadBackups]);

  function toggle(key: string) { invalidatePlan(); setSelected((prev) => { const next = new Set(prev); if (next.has(key)) next.delete(key); else next.add(key); return next; }); }
  function selectAllClient(id: string) {
    if (!matrix) return; invalidatePlan();
    const keys = matrix.rows.filter((r) => r.cells.some((c) => c.clientId === id && c.state !== "unsupported")).map((r) => `${r.name}::${id}`);
    setSelected((prev) => { const next = new Set(prev); const all = keys.every((k) => next.has(k)); keys.forEach((k) => all ? next.delete(k) : next.add(k)); return next; });
  }
  async function makePlan() {
    if (!contextReady || applying) return; invalidatePlan(); const revision = planRevision.current;
    setLoading(true); setError(null);
    const request: PlanInput = { catalogPath, selection: selectionFromCells(selected), scope, projectRoot, mode, allowCommentStrip: false };
    try { const p = await api.plan(request); if (revision !== planRevision.current) return; setPlan(p); setPlanRequest(request); setPage("diff"); }
    catch (e) { if (revision === planRevision.current) setError(String(e)); }
    finally { setLoading(false); }
  }
  async function applyPlan() {
    if (!planRequest || !plan?.writable || applying) return; setApplying(true); setError(null);
    try {
      const r = await api.apply(planRequest, false); setReceipt(r);
      setToast(`已写入 ${r.written} 个文件${r.failed ? `，${r.failed} 项失败，请查看逐项结果和备份` : ""}`);
      await refresh(catalogPath, scope, projectRoot); await loadBackups(); setPage("backups");
    } catch (e) { setError(String(e)); } finally { setApplying(false); }
  }
  async function changeProject(path: string) {
    if (!settings || !settingsValid || !canLeave()) return;
    setChoosing(true); setError(null);
    try {
      // 先由后端验证目录再保存，非法输入不会替换上次有效目录。
      await api.detect("project", path);
      const next = { ...settings, lastProjectDir: path };
      const warnings = await api.saveSettings(next);
      setSettings(next); setProject(path); setProjectInput(path); setDirty(false);
      if (warnings.length) setToast(warnings.join("；"));
      await refresh(catalogPath, scope, path);
    } catch (e) { setError(String(e)); } finally { setChoosing(false); }
  }
  async function chooseProject() {
    setChoosing(true);
    try { const path = await api.chooseDirectory(project); if (path) await changeProject(path); }
    catch (e) { setError(String(e)); } finally { setChoosing(false); }
  }
  function changeScope(next: Scope) {
    if (!canLeave()) return; setDirty(false); setScope(next); setToast(null); refresh(catalogPath, next, project);
  }
  async function useCatalog(create = false) {
    if (!canLeave() || !pathInput.trim()) return;
    try {
      if (create) await api.createCatalog(pathInput.trim());
      setDirty(false); setCatalogPath(pathInput.trim()); invalidatePlan();
      await Promise.all([loadCatalog(pathInput.trim()), refresh(pathInput.trim(), scope, projectRoot)]);
      if (create) setPage("catalog");
    } catch (e) { setError(String(e)); }
  }
  const busy = applying || choosing || !settingsValid;
  const contextLocked = busy || dirty;
  return <div className="shell"><aside className="sidebar"><div className="brand"><span className="brand-mark">M</span> MCP Hub<small>一份配置，连接你的 Agent</small></div>
    <nav className="nav">{NAV.map((n) => <button key={n.id} disabled={applying} className={`nav-item${page === n.id ? " active" : ""}`} onClick={() => navigate(n.id)}><span>{n.icon}</span>{n.label}</button>)}</nav>
    <div className="nav nav-secondary"><span className="nav-caption">配置工具</span>{([{ id: "catalog", label: "MCP配置" }, { id: "matrix", label: "客户端配置" }, { id: "diff", label: "差异预览" }] as const).map((n) => <button key={n.id} className={`nav-item${page === n.id ? " active" : ""}`} disabled={applying} onClick={() => navigate(n.id)}>{n.label}{n.id === "diff" && plan ? <span className="count">{plan.writable}</span> : null}</button>)}</div>
    <div className="sidebar-footer">{info ? <>v{info.version} · {info.platform}<br />数据目录 {info.dataDir}</> : "正在读取环境…"}<p>所有写入都从预览开始</p></div></aside>
    <main className="main"><div className="topbar"><div className="scope-control"><span className="label">配置范围</span><select aria-label="配置范围" className="select" disabled={contextLocked} value={scope} onChange={(e) => changeScope(e.target.value as Scope)}>
      <option value="user">全局配置 · 当前用户跨项目</option><option value="project">项目配置 · 仅当前项目</option></select></div>
      <span className="label">写入策略</span><select aria-label="写入策略" className="select" value={mode} disabled={contextLocked} onChange={(e) => { invalidatePlan(); setMode(e.target.value as MergeMode); }}><option value="merge">字段合并</option><option value="replace">整条替换</option></select>
      <div className="spacer" />{isMock && <span className="pill warn">演示数据 · 不写入磁盘</span>}<button className="btn" disabled={contextLocked || loading || !contextReady} onClick={() => { if (canLeave()) { setDirty(false); refresh(catalogPath, scope, projectRoot); } }}>刷新</button></div>
      {scope !== "user" && <div className="project-bar" aria-label="项目目录选择"><span className="label">项目目录</span><input aria-label="项目目录" className="input grow mono" value={projectInput} placeholder="选择或输入项目绝对目录" disabled={contextLocked} onChange={(e) => setProjectInput(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") changeProject(projectInput); }} />
        <button className="btn" disabled={contextLocked} onClick={chooseProject}>{choosing ? "处理中…" : "选择目录…"}</button><button className="btn" disabled={contextLocked || !projectInput.trim()} onClick={() => changeProject(projectInput)}>使用此目录</button>{project && projectInput !== project && <span className="pill warn">目录尚未应用</span>}</div>}
      <div className={page === "matrix" ? "content fixed" : page === "workbench" ? "content workbench-page" : "content"}>
        {error && <Banner tone="danger"><div><b>需要处理</b><div className="mono">{error}</div></div></Banner>}
        {toast && <Banner tone="ok">{toast}</Banner>}
        {!contextReady && settingsValid && scope !== "user" && <Banner tone="warn">请先选择有效项目目录。项目配置不会使用软件的启动目录。</Banner>}
        {page !== "settings" && <div className="catalog-bar"><span>配置库</span><input aria-label="当前配置库文件" className="input grow mono" value={pathInput} disabled={contextLocked} onChange={(e) => setPathInput(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") useCatalog(); }} />
          <button className="btn small" disabled={contextLocked} onClick={() => useCatalog()}>加载</button>{!catalog && <button className="btn small" disabled={contextLocked} onClick={() => useCatalog(true)}>新建空配置库</button>}</div>}
        {page === "workbench" && <Workbench matrix={matrix} entries={entries} clients={clients} selected={selected} onToggle={toggle} onPlan={makePlan} onEdit={() => navigate("catalog")} loading={loading || !contextReady} />}
        {page === "matrix" && <MatrixPage matrix={matrix} selected={selected} onToggle={toggle} onSelectAllClient={selectAllClient} onPlan={makePlan} loading={loading || !contextReady} />}
        {page === "catalog" && <CatalogPage entries={entries} issues={catalog?.issues ?? []} onDirty={setDirty} onSave={async (list) => {
          if (!settingsValid) throw new Error("请先修复全局设置");
          const warnings = await api.saveCatalog(catalogPath, { version: 1, entries: list }); setDirty(false);
          await loadCatalog(catalogPath); await refresh(catalogPath, scope, projectRoot); return warnings;
        }} />}
        {page === "diff" && <DiffPage plan={plan} applying={applying} onApply={applyPlan} />}
        {page === "detect" && (contextReady ? <DetectPage key={`${scope}:${projectRoot}:${catalogPath}`} clients={clients} scope={scope} projectRoot={projectRoot}
          catalogPath={catalogPath} onDirty={setDirty} onBusy={setApplying} onImported={async (result) => {
            invalidatePlan(); setDirty(false);
            await loadCatalog(catalogPath); await refresh(catalogPath, scope, projectRoot); await loadBackups();
            setToast(`已导入 ${result.imported} 项到配置库；源 Agent 文件未修改${result.warnings.length ? `；${result.warnings.join("；")}` : ""}`);
          }} /> : <p>选择项目目录后即可读取当前项目下的 Agent MCP。</p>)}
        {page === "settings" && settings && <SettingsPage key={settingsValid ? "valid" : "recovery"} settings={settings} dataDir={info?.dataDir ?? "~/.mcp-hub"} onDirty={setDirty} onSave={async (next) => {
          const warnings = await api.saveSettings(next); setSettings(next); setSettingsValid(true); setDirty(false); invalidatePlan();
          setToast(warnings.length ? warnings.join("；") : "全局设置已保存"); setError(null);
          if (!settingsValid) { setCatalogPath(next.catalogPath); setPathInput(next.catalogPath); setScope(next.defaultScope); setMode(next.defaultMode); setProject(next.lastProjectDir); setProjectInput(next.lastProjectDir ?? ""); await loadCatalog(next.catalogPath); await refresh(next.catalogPath, next.defaultScope, next.lastProjectDir); }
          await loadBackups();
        }} />}
        {page === "backups" && <BackupsPage backups={backups} receipt={receipt} onRestore={async (id) => {
          if (!window.confirm(`回滚将覆盖备份 ${id} 涉及文件的当前内容，确认继续？`)) return;
          setApplying(true); try { const n = await api.restore(id); setToast(`已回滚 ${id}，恢复 ${n} 个文件`); await refresh(catalogPath, scope, projectRoot); await loadCatalog(catalogPath); await loadBackups(); } catch (e) { setError(String(e)); } finally { setApplying(false); }
        }} />}
      </div></main></div>;
}
