import { useCallback, useEffect, useRef, useState } from "react";
import { api, isMock } from "./api";
import { selectionFromCells } from "./selection";
import { fileName } from "./library";
import { ACCENT_PRESETS, readInitialAccent, readInitialTheme, type AccentId, type ThemeMode } from "./theme";
import type { AppInfoDto, BackupDto, CatalogDto, ClientDto, GuiSettings, MatrixDto, MergeMode, PlanDto, PlanInput, ReceiptDto, Scope } from "./types";
import type { Entry } from "./pages/Catalog";
import { MatrixPage } from "./pages/Matrix";
import { LibraryPage } from "./pages/Library";
import { EntryEditor } from "./pages/EntryEditor";
import { DiffPage } from "./pages/Diff";
import { BackupsPage, DetectPage } from "./pages/Panels";
import { SettingsPage } from "./pages/Settings";
import { Banner } from "./components/ui";
import { Icon, type IconName } from "./components/Icon";
import { Dialog } from "./components/Dialog";
import { LocationForm } from "./components/LocationForm";

type Page = "library" | "agents" | "history" | "settings";
type Overlay = { kind: "editor"; entry: Entry | null } | { kind: "library" | "project" | "matrix" | "preview" } | null;
type Feedback = { tone: "ok" | "warn" | "danger"; text: string } | null;
const NAV: { id: Page; label: string; icon: IconName; hint: string }[] = [
  { id: "library", label: "我的 MCP", icon: "library", hint: "管理配置与分发" },
  { id: "agents", label: "Agents", icon: "agents", hint: "查看与导入已有配置" },
  { id: "history", label: "变更历史", icon: "history", hint: "写入结果与备份恢复" },
];
const deepLink = new URLSearchParams(window.location.search).get("page") || "library";
const PAGE_ALIASES: Record<string, Page> = { library: "library", workbench: "library", catalog: "library", matrix: "library", diff: "library", agents: "agents", detect: "agents", history: "history", backups: "history", settings: "settings" };

export default function App() {
  const [page, setPage] = useState<Page>(PAGE_ALIASES[deepLink] ?? "library");
  const [overlay, setOverlay] = useState<Overlay>(deepLink === "matrix" ? { kind: "matrix" } : null);
  const [settings, setSettings] = useState<GuiSettings | null>(null);
  const [settingsValid, setSettingsValid] = useState(false);
  const [catalogPath, setCatalogPath] = useState("");
  const [catalog, setCatalog] = useState<CatalogDto | null>(null);
  const [catalogState, setCatalogState] = useState<"loading" | "ready" | "missing" | "error">("loading");
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [scope, setScope] = useState<Scope>("user");
  const [project, setProject] = useState<string | null>(null);
  const [mode, setMode] = useState<MergeMode>("merge");
  const [allowCommentStrip, setAllowCommentStrip] = useState(false);
  const [info, setInfo] = useState<AppInfoDto | null>(null);
  const [matrix, setMatrix] = useState<MatrixDto | null>(null);
  const [clients, setClients] = useState<ClientDto[] | null>(null);
  const [clientError, setClientError] = useState<string | null>(null);
  const [backups, setBackups] = useState<BackupDto[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [focus, setFocus] = useState<string | null>(null);
  const [plan, setPlan] = useState<PlanDto | null>(null);
  const [planRequest, setPlanRequest] = useState<PlanInput | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [receipt, setReceipt] = useState<ReceiptDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [planning, setPlanning] = useState(false);
  const [writing, setWriting] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [feedback, setFeedback] = useState<Feedback>(null);
  const [theme, setTheme] = useState<ThemeMode>(readInitialTheme);
  const [accent, setAccent] = useState<AccentId>(readInitialAccent);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const paletteRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try { localStorage.setItem("mcp-hub-theme", theme); } catch { /* ignore */ }
  }, [theme]);
  useEffect(() => {
    document.documentElement.dataset.accent = accent;
    try { localStorage.setItem("mcp-hub-accent", accent); } catch { /* ignore */ }
  }, [accent]);
  useEffect(() => {
    if (!paletteOpen) return;
    const onPointerDown = (event: MouseEvent) => {
      if (paletteRef.current && !paletteRef.current.contains(event.target as Node)) setPaletteOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setPaletteOpen(false);
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [paletteOpen]);

  const readRevision = useRef(0);
  const planRevision = useRef(0);
  const previewId = useRef<string | null>(null);
  const planningRequest = useRef(0);
  const backupRevision = useRef(0);
  const entries = (catalog?.catalog?.entries ?? []) as Entry[];
  const editingEntry = overlay?.kind === "editor" ? overlay.entry : null;
  const projectRoot = scope === "user" ? null : project;
  const contextReady = settingsValid && (scope === "user" || !!project);
  const busy = writing || !settingsValid;
  const contextLocked = busy || dirty || overlay !== null;
  const currentAccent = ACCENT_PRESETS.find((item) => item.id === accent) ?? ACCENT_PRESETS[0];

  const invalidatePlan = useCallback(() => {
    planRevision.current++;
    const id = previewId.current; previewId.current = null;
    if (id) api.cancelPlan(id).catch(() => {});
    setPlan(null); setPlanRequest(null); setPlanError(null);
  }, []);
  function canLeave() { return !dirty || window.confirm("有未保存的修改，确认放弃这些修改？"); }
  function navigate(next: Page) {
    if (writing || !canLeave()) return;
    invalidatePlan(); setDirty(false); setPage(next); setOverlay(null); setFeedback(null); setPaletteOpen(false);
  }
  function closeOverlay() {
    if (writing || !canLeave()) return;
    if (overlay?.kind === "preview") { invalidatePlan(); setAllowCommentStrip(false); }
    setOverlay(null); setDirty(false);
  }
  function openOverlay(value: Exclude<Overlay, null>) {
    if (writing || !settingsValid || !canLeave()) return;
    invalidatePlan(); setDirty(false); setOverlay(value); setPaletteOpen(false);
  }
  function openEditor(entry: Entry | null) { if (!busy) { setFeedback(null); openOverlay({ kind: "editor", entry }); } }
  const refresh = useCallback(async (path: string, currentScope: Scope, root: string | null) => {
    const revision = ++readRevision.current;
    invalidatePlan(); setLoading(true); setSelected(new Set()); setMatrix(null); setClientError(null);
    setClients(null); setCatalogError(null);
    const effectiveRoot = currentScope === "user" ? null : root;
    const canDetect = currentScope === "user" || !!effectiveRoot;
    const [library, detected] = await Promise.allSettled([api.loadCatalog(path), canDetect ? api.detect(currentScope, effectiveRoot) : Promise.resolve(null)]);
    if (revision !== readRevision.current) return;
    let current: CatalogDto | null = null;
    if (library.status === "fulfilled") {
      current = library.value; setCatalog(current); setCatalogState(current.exists === false ? "missing" : "ready");
    } else { setCatalog(null); setCatalogState("error"); setCatalogError(String(library.reason)); }
    if (detected.status === "fulfilled") setClients(detected.value);
    else { setClients(null); setClientError(String(detected.reason)); }
    if (current && current.exists !== false && canDetect) {
      try { const value = await api.matrix(path, currentScope, effectiveRoot); if (revision === readRevision.current) setMatrix(value); }
      catch (e) { if (revision === readRevision.current) setClientError(String(e)); }
    }
    if (revision === readRevision.current) setLoading(false);
  }, [invalidatePlan]);
  const loadBackups = useCallback(async () => {
    const revision = ++backupRevision.current;
    try { const value = await api.backups(); if (revision === backupRevision.current) setBackups(value); }
    catch (e) { if (revision === backupRevision.current) { setBackups([]); setFeedback({ tone: "warn", text: `无法读取备份：${String(e)}` }); } }
  }, []);
  useEffect(() => {
    let active = true;
    api.appInfo().then((value) => { if (active) setInfo(value); }).catch(() => {});
    (async () => {
      try {
        const value = await api.loadSettings(); if (!active) return;
        setSettings(value); setSettingsValid(true); setCatalogPath(value.catalogPath); setScope(value.defaultScope);
        setMode(value.defaultMode); setProject(value.lastProjectDir);
        await Promise.all([refresh(value.catalogPath, value.defaultScope, value.lastProjectDir), loadBackups()]);
      } catch (e) {
        if (!active) return;
        setFeedback({ tone: "danger", text: String(e) }); setLoading(false); setSettingsValid(false); setPage("settings"); setOverlay(null);
        try { const defaults = await api.defaultSettings(); if (active) setSettings(defaults); } catch { /* 保留原始设置错误 */ }
      }
    })();
    return () => {
      active = false; readRevision.current++; backupRevision.current++; planRevision.current++;
      const id = previewId.current; previewId.current = null;
      if (id) api.cancelPlan(id).catch(() => {});
    };
  }, [refresh, loadBackups]);
  useEffect(() => {
    const warn = (event: BeforeUnloadEvent) => { if (dirty || writing) { event.preventDefault(); event.returnValue = ""; } };
    window.addEventListener("beforeunload", warn); return () => window.removeEventListener("beforeunload", warn);
  }, [dirty, writing]);
  useEffect(() => { if (page === "history" && settingsValid) loadBackups(); }, [page, settingsValid, loadBackups]);

  function toggle(key: string) {
    if (writing) return;
    invalidatePlan(); setSelected((previous) => { const next = new Set(previous); if (next.has(key)) next.delete(key); else next.add(key); return next; });
  }
  function selectAllClient(id: string) {
    if (!matrix || writing) return; invalidatePlan();
    const keys = matrix.rows.filter((row) => row.cells.some((cell) => cell.clientId === id && cell.state !== "unsupported")).map((row) => `${row.name}::${id}`);
    setSelected((previous) => { const next = new Set(previous); const all = keys.every((key) => next.has(key)); keys.forEach((key) => all ? next.delete(key) : next.add(key)); return next; });
  }
  function changeScope(next: Scope) {
    if (contextLocked || !canLeave()) return;
    setScope(next); setFeedback(null); refresh(catalogPath, next, project);
    if (next !== "user" && !project) setOverlay({ kind: "project" });
  }
  function changeMode(next: MergeMode) { invalidatePlan(); setMode(next); }
  function changeAllowCommentStrip(next: boolean) { invalidatePlan(); setAllowCommentStrip(next); }
  async function makePlan() {
    if (!contextReady || !selected.size || writing || planning) return;
    invalidatePlan(); const revision = planRevision.current; const requestId = ++planningRequest.current;
    setPlanning(true); setFeedback(null);
    const request: PlanInput = { catalogPath, selection: selectionFromCells(selected), scope, projectRoot, mode, allowCommentStrip };
    try {
      const value = await api.plan(request);
      if (!value.previewId) throw new Error("预览缺少一致性凭证，请更新配套桌面端后重新预览");
      if (revision !== planRevision.current) { api.cancelPlan(value.previewId).catch(() => {}); return; }
      previewId.current = value.previewId;
      setPlan(value); setPlanRequest(request); setOverlay({ kind: "preview" });
    } catch (e) { if (revision === planRevision.current) { setPlanError(String(e)); setFeedback({ tone: "danger", text: String(e) }); } }
    finally { if (requestId === planningRequest.current) setPlanning(false); }
  }
  async function applyPlan() {
    if (!planRequest || !plan?.writable || !plan.previewId || writing) return;
    setWriting(true); setFeedback(null);
    try {
      const value = await api.apply(planRequest, false, plan.previewId); setReceipt(value);
      setOverlay(null); setAllowCommentStrip(false); await refresh(catalogPath, scope, projectRoot); await loadBackups(); setPage("history");
      setFeedback({ tone: value.failed || value.blocked ? "warn" : "ok", text: `已写入 ${value.written} 个文件${value.failed ? `，${value.failed} 项失败，请检查逐项结果` : value.blocked ? `，${value.blocked} 项被阻止` : "；备份可在下方恢复"}` });
    } catch (e) {
      invalidatePlan();
      const text = `确认写入已停止：${String(e)}。请重新预览后再确认。`;
      setPlanError(text); setFeedback({ tone: "danger", text });
    }
    finally { setWriting(false); }
  }
  async function saveEntries(list: Entry[], renames?: Record<string, string>) {
    if (!settingsValid || catalogState === "error") throw new Error("请先修复或切换配置库位置");
    const wasMissing = catalogState === "missing";
    if (wasMissing) await api.createCatalog(catalogPath);
    const warnings = await api.saveCatalog(catalogPath, { version: 1, entries: list }, wasMissing ? undefined : catalog?.revisionId, renames);
    await refresh(catalogPath, scope, projectRoot); await loadBackups();
    setDirty(false); setOverlay(null); setFeedback({ tone: warnings.length ? "warn" : "ok", text: warnings.length ? `配置库已保存；${warnings.join("；")}` : "已保存到 MCP 库，尚未修改任何 Agent。选择目标后可预览变更。" });
  }
  const currentTitle = NAV.find((item) => item.id === page)?.label ?? "软件设置";
  const libraryTitle = catalogPath && fileName(catalogPath) !== "catalog.yaml" ? fileName(catalogPath) : "默认配置库";
  return <div className="shell">
    <aside className="sidebar"><div className="brand"><span className="brand-mark"><Icon name="library" size={22} /></span><div>MCP Hub<small>统一管理 MCP 配置</small></div></div>
      <button className="library-switch" disabled={busy || dirty || !!overlay} onClick={() => openOverlay({ kind: "library" })} aria-label="管理配置库"><Icon name="folder" /><span><small>当前配置库</small><b>{libraryTitle}</b></span><Icon name="chevron" size={15} /></button>
      <nav className="nav" aria-label="主导航">{NAV.map((item) => <button key={item.id} aria-label={item.label} disabled={writing} className={`nav-item${page === item.id ? " active" : ""}`} aria-current={page === item.id ? "page" : undefined} onClick={() => navigate(item.id)}><Icon name={item.icon} /><span>{item.label}<small>{item.hint}</small></span>{item.id === "library" && !!entries.length && <span className="count">{entries.length}</span>}</button>)}</nav>
      <div className="sidebar-footer"><button aria-label="软件设置" className={`nav-item${page === "settings" ? " active" : ""}`} disabled={writing} onClick={() => navigate("settings")}><Icon name="settings" /><span>软件设置</span></button><div className="version-line"><span className="status-dot" />{info ? `v${info.version}` : "正在初始化…"}<span>本地配置工具</span></div></div>
    </aside>
    <main className="main"><header className="topbar"><span className="breadcrumb">工作空间<span>/</span><b>{currentTitle}</b></span><div className="spacer" />
      {page !== "settings" && <div className="workspace-context"><label><span className="sr-only">配置范围</span><select aria-label="配置范围" className="select scope-select" disabled={contextLocked} value={scope} onChange={(event) => changeScope(event.target.value as Scope)}><option value="user">全局配置 · 当前用户</option><option value="project">项目配置 · 当前目录</option><option value="local">本地配置 · 项目私有</option></select></label>
        {scope !== "user" && <button className={`btn project-button${!project ? " needs-project" : ""}`} aria-label="选择项目目录" disabled={contextLocked} onClick={() => openOverlay({ kind: "project" })}><Icon name="folder" size={16} />{project ? fileName(project) : "选择项目目录"}</button>}
      </div>}
      {isMock && <span className="pill warn">演示数据</span>}
      <div className="theme-picker" ref={paletteRef}>
        <button
          className={`btn ghost icon-button palette-trigger${paletteOpen ? " active" : ""}`}
          aria-label="切换主题配色"
          title={`主题配色：${currentAccent.label}`}
          aria-expanded={paletteOpen}
          onClick={() => setPaletteOpen((prev) => !prev)}
        >
          <Icon name="palette" />
          <span className="palette-dot" style={{ background: currentAccent.swatch }} aria-hidden="true" />
        </button>
        {paletteOpen && (
          <div className="theme-popover" role="dialog" aria-label="界面主题配色">
            <div className="theme-popover-section">
              <span className="theme-popover-title">外观模式</span>
              <div className="theme-mode-switch">
                <button className={`theme-mode-btn${theme === "light" ? " active" : ""}`} onClick={() => setTheme("light")}><Icon name="sun" size={15} />浅色</button>
                <button className={`theme-mode-btn${theme === "dark" ? " active" : ""}`} onClick={() => setTheme("dark")}><Icon name="moon" size={15} />深色</button>
              </div>
            </div>
            <div className="theme-popover-section">
              <span className="theme-popover-title">主题色调</span>
              <div className="accent-grid">
                {ACCENT_PRESETS.map((item) => (
                  <button
                    key={item.id}
                    className={`accent-chip${accent === item.id ? " active" : ""}`}
                    onClick={() => setAccent(item.id)}
                    aria-pressed={accent === item.id}
                  >
                    <span className="accent-swatch" style={{ background: item.swatch }} aria-hidden="true" />
                    <span>{item.label}</span>
                  </button>
                ))}
              </div>
            </div>
          </div>
        )}
      </div>
      <button className="btn ghost icon-button" aria-label={theme === "dark" ? "切换到浅色外观" : "切换到深色外观"} title={theme === "dark" ? "切换到浅色外观" : "切换到深色外观"} onClick={() => setTheme((prev) => prev === "dark" ? "light" : "dark")}><Icon name={theme === "dark" ? "sun" : "moon"} /></button>
      <button className="btn ghost icon-button" aria-label="刷新配置与状态" title="刷新配置与状态" disabled={contextLocked || loading} onClick={() => refresh(catalogPath, scope, projectRoot)}><Icon name="refresh" /></button>
    </header>
    <div className={`content${page === "library" ? " library-content" : ""}`}>
      {feedback && <div className="feedback-wrap" role="status" aria-live="polite"><Banner tone={feedback.tone}><span>{feedback.text}</span><button className="btn ghost icon-button small" aria-label="关闭提示" onClick={() => setFeedback(null)}><Icon name="close" size={15} /></button></Banner></div>}
      {clientError && page !== "settings" && <Banner tone="warn"><span>当前范围的 Agent 状态未完整读取：{clientError}</span>{scope !== "user" && <button className="text-button" disabled={contextLocked} onClick={() => openOverlay({ kind: "project" })}>检查项目目录</button>}</Banner>}
      {!contextReady && settingsValid && scope !== "user" && <Banner tone="info"><span>先选择项目目录，即可读取项目或本地配置并选择分发目标。</span><button className="text-button" disabled={contextLocked} onClick={() => openOverlay({ kind: "project" })}>选择项目目录</button></Banner>}
      {page === "library" && !!catalog?.issues.length && <Banner tone="warn"><div>{catalog.issues.map((issue) => <p key={issue}>{issue}</p>)}</div></Banner>}
      {page === "library" && <LibraryPage entries={entries} matrix={matrix} clients={clients} scope={scope} selected={selected} focus={focus} onFocus={setFocus} onToggle={toggle}
        onAdd={() => openEditor(null)} onEdit={openEditor} onImport={() => navigate("agents")} onManage={() => openOverlay({ kind: "library" })}
        onPlan={makePlan} onMatrix={() => openOverlay({ kind: "matrix" })} onScopeChange={changeScope} mode={mode} onModeChange={changeMode}
        loading={loading} planning={planning} blocked={busy || loading} missing={catalogState === "missing"} error={catalogError} onRetry={() => refresh(catalogPath, scope, projectRoot)} />}
      {page === "agents" && <DetectPage key={`${scope}:${projectRoot}:${catalogPath}`} clients={clients} scope={scope} projectRoot={projectRoot} catalogPath={catalogPath}
        loading={loading} contextReady={contextReady} busy={writing} onChooseProject={() => openOverlay({ kind: "project" })} onDirty={setDirty} onBusy={setWriting} onImported={async (value) => {
          setDirty(false); invalidatePlan(); await refresh(catalogPath, scope, projectRoot); await loadBackups(); setPage("library"); setFocus(null);
          setFeedback({ tone: value.warnings.length ? "warn" : "ok", text: `已导入 ${value.imported} 项到 MCP 库，源 Agent 文件未修改。${value.warnings.length ? value.warnings.join("；") : "下一步：选择配置并应用到目标 Agent。"}` });
        }} />}
      {page === "history" && <BackupsPage backups={backups} receipt={receipt} busy={writing} onRestore={async (id) => {
        if (writing) return;
        if (!window.confirm(`恢复备份 ${id} 将覆盖涉及文件的当前内容。请先另存后续编辑，确认继续？`)) return;
        setWriting(true); setFeedback(null); try { const restored = await api.restore(id); await refresh(catalogPath, scope, projectRoot); await loadBackups(); setReceipt(null); setFeedback({ tone: "ok", text: `已恢复 ${restored} 个文件，请在目标 Agent 中核对配置。` }); }
        catch (e) { setFeedback({ tone: "danger", text: String(e) }); } finally { setWriting(false); }
      }} />}
      {page === "settings" && settings && <SettingsPage key={settingsValid ? "valid" : "recovery"} settings={settings} dataDir={info?.dataDir ?? "~/.mcp-hub"} theme={theme} accent={accent} onThemeChange={setTheme} onAccentChange={setAccent} onDirty={setDirty} onBusy={setWriting} onSave={async (value) => {
        const warnings = await api.saveSettings(value); setSettings(value); setSettingsValid(true); setDirty(false); invalidatePlan();
        if (!settingsValid) { setCatalogPath(value.catalogPath); setScope(value.defaultScope); setProject(value.lastProjectDir); setMode(value.defaultMode); await refresh(value.catalogPath, value.defaultScope, value.lastProjectDir); }
        await loadBackups(); setFeedback({ tone: warnings.length ? "warn" : "ok", text: warnings.length ? warnings.join("；") : "设置已保存。默认配置库、范围和写入策略将在下次启动时使用；当前工作范围保持不变。" });
      }} />}
    </div></main>
    {overlay && <Dialog key={overlay.kind} title={overlay.kind === "editor" ? overlay.entry ? `编辑 ${overlay.entry.name}` : "添加 MCP" : overlay.kind === "library" ? "管理配置库" : overlay.kind === "project" ? "选择项目目录" : overlay.kind === "matrix" ? "批量选择" : "预览变更"}
      wide={overlay.kind === "matrix" || overlay.kind === "preview"} busy={writing} onClose={closeOverlay}>
      {overlay.kind === "editor" && <EntryEditor entry={overlay.entry} entries={entries} onDirty={setDirty} onBusy={setWriting} onClose={closeOverlay}
        onSave={async (next) => {
          const list = editingEntry ? entries.map((item) => item.name === editingEntry.name ? next : item) : [...entries, next];
          const renames = editingEntry && editingEntry.name !== next.name ? { [next.name]: editingEntry.name } : undefined;
          await saveEntries(list, renames);
          setFocus(next.name);
        }}
        onRemove={editingEntry ? async () => { await saveEntries(entries.filter((item) => item.name !== editingEntry.name)); setFocus(null); } : null} />}
      {overlay.kind === "library" && <LocationForm kind="library" initial={catalogPath} onClose={closeOverlay} onBusy={setWriting} onUse={async (path, remember) => {
        await api.loadCatalog(path);
        let warnings: string[] = [];
        if (remember && settings) { const next = { ...settings, catalogPath: path }; warnings = await api.saveSettings(next); setSettings(next); }
        setCatalogPath(path); setFocus(null); await refresh(path, scope, projectRoot); setOverlay(null);
        setFeedback({ tone: warnings.length ? "warn" : "ok", text: warnings.length ? warnings.join("；") : "已切换配置库，Agent 文件保持不变。" });
      }} />}
      {overlay.kind === "project" && <LocationForm kind="project" initial={project ?? ""} onClose={closeOverlay} onBusy={setWriting} onUse={async (path) => {
        if (!settings) throw new Error("软件设置尚未加载");
        await api.detect(scope === "user" ? "project" : scope, path); const next = { ...settings, lastProjectDir: path }; const warnings = await api.saveSettings(next);
        setSettings(next); setProject(path); await refresh(catalogPath, scope, path); setOverlay(null);
        setFeedback({ tone: warnings.length ? "warn" : "ok", text: warnings.length ? warnings.join("；") : "项目目录已更新，请为当前项目选择分发目标。" });
      }} />}
      {overlay.kind === "matrix" && <><p className="dialog-intro">高级批量工具。每个单元格表示一组 MCP 与 Agent，未勾选的条目保持不变。</p><MatrixPage matrix={matrix} selected={selected} onToggle={toggle} onSelectAllClient={selectAllClient} onPlan={makePlan} loading={planning || !contextReady} /></>}
      {overlay.kind === "preview" && <DiffPage plan={plan} error={planError} applying={writing} onApply={applyPlan} onBack={closeOverlay} mode={mode} onModeChange={changeMode} allowCommentStrip={allowCommentStrip} onAllowCommentStripChange={changeAllowCommentStrip} onReplan={makePlan} planning={planning} />}
    </Dialog>}
  </div>;
}
