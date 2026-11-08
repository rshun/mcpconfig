import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { BackupDto, ClientDto, ExistingConfigDto, ImportResult, ReceiptDto, Scope } from "../types";
import { Banner, Empty, Pill } from "../components/ui";
import { AgentBadge } from "../components/AgentBadge";
import { Dialog } from "../components/Dialog";
import { Icon } from "../components/Icon";
import { ImportPanel } from "./Import";

export function DetectPage({ clients, scope, projectRoot, catalogPath, loading, contextReady, busy, onChooseProject, onDirty, onBusy, onImported }: {
  clients: ClientDto[] | null; scope: Scope; projectRoot: string | null; catalogPath: string; loading: boolean; contextReady: boolean; busy: boolean;
  onChooseProject: () => void; onDirty: (dirty: boolean) => void; onBusy: (busy: boolean) => void; onImported: (result: ImportResult) => Promise<void>;
}) {
  const [config, setConfig] = useState<ExistingConfigDto | null>(null);
  const [reading, setReading] = useState<string | null>(null);
  const [active, setActive] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [importClient, setImportClient] = useState<string | null>(null);
  const revision = useRef(0);
  useEffect(() => {
    revision.current++; setConfig(null); setReading(null); setError(null); setActive(null); setImportClient(null);
    return () => { revision.current++; };
  }, [clients, scope, projectRoot]);
  async function readConfig(clientId: string) {
    if (busy || importClient) return;
    const current = ++revision.current; setActive(clientId); setReading(clientId); setConfig(null); setError(null);
    try { const value = await api.readClientConfig(clientId, scope, projectRoot); if (current === revision.current) setConfig(value); }
    catch (e) { if (current === revision.current) setError(String(e)); }
    finally { if (current === revision.current) setReading(null); }
  }
  const detected = clients?.filter((client) => client.state !== "missing") ?? [];
  const others = clients?.filter((client) => client.state === "missing") ?? [];
  function agentCard(client: ClientDto) {
    return <button key={client.id} className={`agent-choice${active === client.id ? " active" : ""}`} aria-label={`查看 ${client.name} 配置`} aria-pressed={active === client.id}
      disabled={!!client.scopeBlock || !!importClient || busy} onClick={() => readConfig(client.id)}>
      <AgentBadge clientId={client.id} size="lg" /><span><b>{client.name}</b><small>{client.scopeBlock || (client.configured ? `${client.configured} 项已有配置` : client.state === "missing" ? "未检测到 · 可提前配置" : "尚无 MCP 配置")}</small></span><Icon name="chevron" size={15} />
    </button>;
  }
  return <section className="agents-page"><header className="page-heading"><div><span className="eyebrow">YOUR AGENTS</span><h1>Agents</h1><p className="sub">查看已有配置，安全导入到你的 MCP 库。</p></div><span className="quiet-caption"><Icon name="shield" size={16} />读取不会修改文件</span></header>
    {!contextReady ? <div className="welcome-state"><div className="welcome-symbol"><Icon name="folder" size={28} /></div><h2>先选择要查看的项目</h2><p>项目配置只读取指定目录，不会沿用启动目录。</p><button className="btn primary" onClick={onChooseProject}>选择项目目录</button></div>
    : loading && !clients ? <Empty title="正在检测 Agent…" hint="只检查配置位置和安装状态" /> : <div className="agents-workspace"><aside className="agent-list" aria-label="Agent 列表"><div className="section-caption">已检测到 · {detected.length}</div>
      {detected.map(agentCard)}{!detected.length && <p className="field-hint">尚未检测到 Agent，可展开下方列表检查预定义的配置位置。</p>}
      {!!others.length && <details className="other-agents"><summary>其他 Agent（{others.length}）</summary>{others.map(agentCard)}</details>}
    </aside><div className="agent-config-area">
      {error && <Banner tone="danger">{error}</Banner>}
      {reading ? <div className="panel read-placeholder" role="status"><span className="loading-orbit" /><h3>正在读取已有 MCP</h3><p>识别到的凭据会在返回界面前隐藏。</p></div>
      : config ? <section className="panel existing-config" aria-label="已有 MCP 配置"><div className="panel-head"><AgentBadge clientId={config.clientId} size="lg" /><div><span className="eyebrow">EXISTING CONFIGURATION</span><h2>{config.clientName}</h2></div><span className="pill">{config.format}</span><div className="spacer" />
        <button className="btn primary" disabled={!!importClient || !catalogPath || !config.existed || !config.entries.length || busy} onClick={() => setImportClient(config.clientId)}><Icon name="import" size={16} />导入到 MCP 库</button></div>
        <div className="panel-body"><div className="source-path"><span>来源文件</span><code>{config.path}</code></div><p className="field-hint">{config.entries.length} 个 MCP · 配置文件状态，不代表连接已验证</p>
          {!config.existed ? <Empty title="配置文件尚不存在" hint="可以先在 MCP 库添加配置，再应用到这个 Agent。" /> : !config.entries.length ? <Empty title="配置文件中没有 MCP 条目" /> : config.entries.map((entry) => <details className="existing-entry" key={entry.name}><summary><span className="entry-dot" />{entry.name}<span className="muted">查看配置（凭据隐藏）</span></summary><pre>{JSON.stringify(entry.config, null, 2)}</pre></details>)}
          <div className="read-only-note"><Icon name="shield" size={16} /><span>只读查看，普通配置正常显示，识别到的凭据已隐藏。导入会进入独立的选择与预览流程。</span></div>
        </div></section> : !error && <div className="agent-empty"><div className="welcome-symbol"><Icon name="agents" size={32} /></div><h2>选择一个 Agent，查看它的 MCP</h2><p>已有配置可以导入配置库，方便在多个 Agent 之间复用。</p></div>}
    </div></div>}
    {importClient && <Dialog title="导入到 MCP 库" wide busy={busy} onClose={() => { if (!busy) setImportClient(null); }}>
      <ImportPanel clientId={importClient} catalogPath={catalogPath} scope={scope} projectRoot={projectRoot} embedded
        onClose={() => setImportClient(null)} onDirty={onDirty} onBusy={onBusy} onImported={onImported} />
    </Dialog>}
  </section>;
}

export function BackupsPage({ backups, receipt, busy, onRestore }: { backups: BackupDto[] | null; receipt: ReceiptDto | null; busy: boolean; onRestore: (id: string) => void }) {
  return <section className="history-page" aria-busy={busy}><header className="page-heading"><div><span className="eyebrow">CHANGE HISTORY</span><h1>变更历史</h1><p className="sub">核对写入结果，在需要时恢复原配置。</p></div><span className="quiet-caption"><Icon name="history" size={16} />保存前保留原文备份</span></header>
    {receipt && <section className={`receipt-panel${receipt.failed || receipt.blocked ? " has-issues" : ""}`} aria-label="本次写入结果"><div className="receipt-summary"><span className="receipt-icon"><Icon name={receipt.failed || receipt.blocked ? "history" : "check"} size={24} /></span><div><h2>{receipt.failed || receipt.blocked ? "部分目标需要处理" : receipt.written ? "配置已写入" : "本次没有写入"}</h2><p>写入 {receipt.written} 个文件{receipt.failed ? ` · 失败 ${receipt.failed} 项` : ""}{receipt.blocked ? ` · 阻止 ${receipt.blocked} 项` : ""} · 连接状态未验证</p></div></div>
      {receipt.backupId && <p className="field-hint">备份 ID：<code>{receipt.backupId}</code></p>}
      {!!receipt.outcomes.length && <div className="outcome-list">{receipt.outcomes.map((outcome, index) => <div className="outcome-row" key={`${outcome.clientId}-${index}`}><span className={`pill${["failed", "rollbackFailed", "auditFailed"].includes(outcome.status) ? " warn" : ""}`}>{outcome.label || outcome.status}</span><div><b>{outcome.clientId}</b><code>{outcome.path}</code>{outcome.message && <p>{outcome.message}</p>}</div></div>)}</div>}
    </section>}
    {!backups ? <Empty title="正在读取备份…" /> : !backups.length ? <div className="welcome-state compact"><div className="welcome-symbol"><Icon name="history" size={30} /></div><h2>还没有变更记录</h2><p>首次保存配置库或应用到 Agent 后，备份会显示在这里。</p></div>
    : <div className="backup-list">{backups.map((backup) => <article className="backup-card" key={backup.id}><div className="backup-card-top"><span className="backup-icon"><Icon name="folder" size={19} /></span><div><h3>{Number.isNaN(Date.parse(backup.createdAt)) ? backup.createdAt : new Date(backup.createdAt).toLocaleString()}</h3><small>{backup.files} 个文件 · {backup.id}</small></div><div className="spacer" /><button className="btn" disabled={busy} onClick={() => onRestore(backup.id)}><Icon name="history" size={16} />{busy ? "恢复中…" : "恢复此备份"}</button></div>
      <details><summary>查看涉及文件</summary><ul>{backup.paths.map((path) => <li className="mono" key={path}>{path}</li>)}</ul></details></article>)}</div>}
    <p className="history-note">恢复会覆盖相应文件的当前内容，请先另存后续编辑。备份可能包含原始凭据，请妥善保管目录。</p>
  </section>;
}
