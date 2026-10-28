import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, DEFAULT_CATALOG, isMock } from "./api";
import { selectionFromCells } from "./selection";
import type {
  AppInfoDto,
  BackupDto,
  CatalogDto,
  ClientDto,
  MatrixDto,
  MergeMode,
  PlanDto,
  PlanInput,
  ReceiptDto,
  Scope,
} from "./types";
import { MatrixPage } from "./pages/Matrix";
import { CatalogPage, type Entry } from "./pages/Catalog";
import { DiffPage } from "./pages/Diff";
import { BackupsPage, DetectPage } from "./pages/Panels";
import { Banner } from "./components/ui";

type Page = "matrix" | "catalog" | "diff" | "detect" | "backups";

const NAV: { id: Page; label: string }[] = [
  { id: "matrix", label: "客户端配置" },
  { id: "catalog", label: "MCP配置" },
  { id: "diff", label: "差异预览" },
  { id: "detect", label: "环境检测" },
  { id: "backups", label: "备份回滚" },
];

const params =
  typeof window !== "undefined"
    ? new URLSearchParams(window.location.search)
    : new URLSearchParams();

/** 深链：?page=catalog 直接打开某页；?demo=1 自动生成计划（便于逐屏评审） */
const initialPage = (params.get("page") as Page) || "matrix";
const demoMode = params.get("demo") === "1";

export default function App() {
  const [page, setPage] = useState<Page>(initialPage);
  const [catalogPath, setCatalogPath] = useState(DEFAULT_CATALOG);
  const [pathInput, setPathInput] = useState(DEFAULT_CATALOG);
  const [scope, setScope] = useState<Scope>("user");
  const [mode, setMode] = useState<MergeMode>("merge");

  const [info, setInfo] = useState<AppInfoDto | null>(null);
  const [matrix, setMatrix] = useState<MatrixDto | null>(null);
  const [clients, setClients] = useState<ClientDto[] | null>(null);
  const [catalog, setCatalog] = useState<CatalogDto | null>(null);
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [backups, setBackups] = useState<BackupDto[] | null>(null);

  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [plan, setPlan] = useState<PlanDto | null>(null);
  const [planRequest, setPlanRequest] = useState<PlanInput | null>(null);
  const planRevision = useRef(0);
  const [receipt, setReceipt] = useState<ReceiptDto | null>(null);

  const [loading, setLoading] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  const invalidatePlan = useCallback(() => {
    planRevision.current += 1;
    setPlan(null);
    setPlanRequest(null);
  }, []);

  const refresh = useCallback(
    async (path: string, sc: Scope) => {
      invalidatePlan();
      setLoading(true);
      setError(null);
      try {
        // 检测和读取已有配置不依赖清单成功加载。
        const [m, c] = await Promise.allSettled([api.matrix(path, sc), api.detect(sc)]);
        if (c.status === "fulfilled") setClients(c.value);
        else setClients(null);
        if (m.status === "fulfilled") setMatrix(m.value);
        else setMatrix(null);
        setSelected(new Set());
        if (m.status === "rejected") throw m.reason;
        if (c.status === "rejected") throw c.reason;
      } catch (e) {
        setError(String(e));
        setMatrix(null);
      } finally {
        setLoading(false);
      }
    },
    [invalidatePlan]
  );

  const loadCatalog = useCallback(async (path: string) => {
    try {
      const c = await api.loadCatalog(path);
      setCatalog(c);
      setEntries((c.catalog?.entries ?? []) as Entry[]);
    } catch (e) {
      setError(String(e));
      setCatalog(null);
      setEntries([]);
    }
  }, []);

  const loadBackups = useCallback(async () => {
    try {
      setBackups(await api.backups());
    } catch {
      setBackups([]);
    }
  }, []);

  useEffect(() => {
    api.appInfo().then(setInfo).catch(() => {});
    refresh(catalogPath, scope);
    loadCatalog(catalogPath);
    loadBackups();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 评审用：带 demo=1 时自动生成全客户端计划，便于截图差异页
  useEffect(() => {
    if (!demoMode || !matrix || plan) return;
    const revision = planRevision.current;
    (async () => {
      try {
        const cells = matrix.rows.flatMap((row) =>
          row.cells
            .filter((cell) => cell.state !== "unsupported")
            .map((cell) => `${row.name}::${cell.clientId}`)
        );
        const request: PlanInput = {
          catalogPath,
          selection: selectionFromCells(cells),
          scope,
          mode,
          allowCommentStrip: false,
        };
        const p = await api.plan(request);
        if (revision !== planRevision.current) return;
        setSelected(new Set(cells));
        setPlan(p);
        setPlanRequest(request);
      } catch {
        /* 忽略 */
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [matrix]);

  useEffect(() => {
    if (page === "backups") loadBackups();
    if (page === "catalog" && !entries) loadCatalog(catalogPath);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page]);

  const toggle = (key: string) => {
    invalidatePlan();
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const selectAllClient = (clientId: string) => {
    if (!matrix) return;
    invalidatePlan();
    const keys = matrix.rows
      .filter((r) => {
        const cell = r.cells.find((c) => c.clientId === clientId);
        return cell && cell.state !== "unsupported";
      })
      .map((r) => `${r.name}::${clientId}`);
    setSelected((prev) => {
      const next = new Set(prev);
      const allOn = keys.every((k) => next.has(k));
      keys.forEach((k) => (allOn ? next.delete(k) : next.add(k)));
      return next;
    });
  };

  const makePlan = async () => {
    invalidatePlan();
    const revision = planRevision.current;
    setLoading(true);
    setError(null);
    try {
      const request: PlanInput = {
        catalogPath,
        selection: selectionFromCells(selected),
        scope,
        mode,
        allowCommentStrip: false,
      };
      const p = await api.plan(request);
      if (revision !== planRevision.current) return;
      setPlan(p);
      setPlanRequest(request);
      setPage("diff");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const applyPlan = async () => {
    if (!planRequest || !plan?.writable || applying) return;
    setApplying(true);
    try {
      const r = await api.apply(planRequest, false);
      setReceipt(r);
      setToast(
        r.failed > 0
          ? `已写入 ${r.written} 个文件，${r.failed} 项失败，请查看逐项结果和备份`
          : `已写入 ${r.written} 个文件`
      );
      await refresh(catalogPath, scope);
      await loadBackups();
      setPage("backups");
    } catch (e) {
      setError(String(e));
    } finally {
      setApplying(false);
    }
  };

  const counts = useMemo(
    () => ({
      matrix: matrix?.rows.length ?? 0,
      catalog: entries?.length ?? 0,
      detect: clients?.filter((c) => c.state === "ready").length ?? 0,
      backups: backups?.length ?? 0,
      diff: plan?.writable ?? 0,
    }),
    [matrix, entries, clients, backups, plan]
  );

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          MCP Hub
          <small>多客户端配置分发</small>
        </div>
        <nav className="nav">
          {NAV.map((n) => (
            <button
              key={n.id}
              className={`nav-item${page === n.id ? " active" : ""}`}
              onClick={() => setPage(n.id)}
            >
              {n.label}
              <span className="count">
                {n.id === "matrix"
                  ? counts.matrix
                  : n.id === "catalog"
                    ? counts.catalog
                    : n.id === "diff"
                      ? counts.diff
                      : n.id === "detect"
                        ? counts.detect
                        : counts.backups}
              </span>
            </button>
          ))}
        </nav>
        <div className="sidebar-footer">
          {info ? (
            <>
              v{info.version} · {info.platform}
              <br />
              数据目录 {info.dataDir}
            </>
          ) : (
            "正在读取环境…"
          )}
        </div>
      </aside>

      <main className="main">
        <div className="topbar">
          <span className="label">清单</span>
          <input
            className="input grow"
            value={pathInput}
            spellCheck={false}
            onChange={(e) => setPathInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                setCatalogPath(pathInput);
                refresh(pathInput, scope);
                loadCatalog(pathInput);
              }
            }}
          />
          <button
            className="btn"
            onClick={() => {
              setCatalogPath(pathInput);
              refresh(pathInput, scope);
              loadCatalog(pathInput);
            }}
          >
            加载
          </button>

          <span className="label">作用域</span>
          <select
            className="select"
            value={scope}
            onChange={(e) => {
              const s = e.target.value as Scope;
              setScope(s);
              refresh(catalogPath, s);
            }}
          >
            <option value="user">用户级</option>
            <option value="project">项目级</option>
            <option value="local">本地级</option>
          </select>

          <span className="label">覆盖</span>
          <select
            className="select"
            value={mode}
            onChange={(e) => {
              invalidatePlan();
              setMode(e.target.value as MergeMode);
            }}
          >
            <option value="merge">字段合并</option>
            <option value="replace">整条替换</option>
          </select>

          <div className="spacer" />
          {isMock && <span className="pill warn">演示数据</span>}
          <button className="btn" onClick={() => refresh(catalogPath, scope)} disabled={loading}>
            {loading ? "刷新中…" : "刷新"}
          </button>
        </div>

        <div className={page === "matrix" ? "content fixed" : "content"}>
          {error && (
            <Banner tone="danger">
              <div>
                <b>出错了</b>
                <div className="mono">{error}</div>
              </div>
            </Banner>
          )}
          {toast && (
            <Banner tone="ok">
              <div>{toast}</div>
            </Banner>
          )}

          {page === "matrix" && (
            <MatrixPage
              matrix={matrix}
              selected={selected}
              onToggle={toggle}
              onSelectAllClient={selectAllClient}
              onPlan={makePlan}
              loading={loading}
            />
          )}

          {page === "catalog" && (
            <CatalogPage
              entries={entries}
              issues={catalog?.issues ?? []}
              onSave={async (list) => {
                const warnings = await api.saveCatalog(catalogPath, { version: 1, entries: list });
                await loadCatalog(catalogPath);
                await refresh(catalogPath, scope);
                return warnings;
              }}
              onDirty={() => {}}
            />
          )}

          {page === "diff" && (
            <DiffPage plan={plan} applying={applying} onApply={applyPlan} />
          )}

          {page === "detect" && <DetectPage key={scope} clients={clients} scope={scope} />}

          {page === "backups" && (
            <BackupsPage
              backups={backups}
              receipt={receipt}
              onRestore={async (id) => {
                try {
                  const n = await api.restore(id);
                  setToast(`已回滚 ${id}，恢复 ${n} 个文件`);
                  await refresh(catalogPath, scope);
                  await loadBackups();
                } catch (e) {
                  setError(String(e));
                }
              }}
            />
          )}
        </div>
      </main>
    </div>
  );
}
