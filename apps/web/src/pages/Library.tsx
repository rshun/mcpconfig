import { useEffect, useRef, useState } from "react";
import { AgentBadge } from "../components/AgentBadge";
import { Icon } from "../components/Icon";
import { scopeLabel, scopeReason, variableNames } from "../library";
import { selectionFromCells } from "../selection";
import type { ClientDto, MatrixDto, MergeMode, Scope } from "../types";
import type { Entry } from "./Catalog";

const CELL = { ok: "已配置且一致", diff: "已有配置，存在差异", absent: "未配置", unsupported: "不支持此范围" };
export function LibraryPage({ entries, matrix, clients, scope, selected, focus, onFocus, onToggle, onAdd, onEdit, onImport, onManage,
  onPlan, onMatrix, onScopeChange, mode, onModeChange, loading, planning, blocked, missing, error, onRetry }: {
  entries: Entry[]; matrix: MatrixDto | null; clients: ClientDto[] | null; scope: Scope; selected: Set<string>; focus: string | null;
  onFocus: (name: string | null) => void; onToggle: (key: string) => void; onAdd: () => void; onEdit: (entry: Entry) => void;
  onImport: () => void; onManage: () => void; onPlan: () => void; onMatrix: () => void; onScopeChange: (scope: Scope) => void;
  mode: MergeMode; onModeChange: (mode: MergeMode) => void; loading: boolean; planning: boolean; blocked: boolean;
  missing: boolean; error: string | null; onRetry: () => void;
}) {
  const [query, setQuery] = useState("");
  const inspector = useRef<HTMLElement>(null);
  const list = entries.filter((entry) => `${entry.name} ${entry.description ?? ""} ${(entry.tags ?? []).join(" ")}`.toLowerCase().includes(query.toLowerCase()));
  const entry = list.find((entry) => entry.name === focus);
  const pairs = selectionFromCells(selected);
  const selectedNames = new Set(Object.values(pairs).flat());
  useEffect(() => {
    if (focus && window.matchMedia("(max-width: 1100px)").matches) inspector.current?.scrollIntoView({ block: "start", behavior: "auto" });
  }, [focus]);

  function target(client: ClientDto, current: Entry) {
    const cell = matrix?.rows.find((row) => row.name === current.name)?.cells.find((cell) => cell.clientId === client.id);
    const reason = scopeReason(current, scope) ?? client.scopeBlock ?? (cell?.state === "unsupported" ? cell.detail || CELL.unsupported : null);
    const key = `${current.name}::${client.id}`;
    return <label className={`agent-target${selected.has(key) ? " chosen" : ""}${reason ? " unavailable" : ""}`} key={client.id}>
      <input type="checkbox" aria-label={`${current.name} → ${client.name}`} checked={selected.has(key)}
        disabled={!!reason || !cell || blocked} onChange={() => onToggle(key)} />
      <AgentBadge clientId={client.id} size="md" />
      <span className="agent-target-copy"><b>{client.name}</b><small>{reason ?? (cell ? CELL[cell.state] : "尚未读取此范围的配置")}</small></span>
      {cell?.state === "ok" && <Icon name="check" size={15} />}
    </label>;
  }
  const visibleClients = clients?.filter((client) => client.state !== "missing") ?? [];
  const otherClients = clients?.filter((client) => client.state === "missing") ?? [];
  return <section className="library-page" aria-label="MCP 配置库">
    <div className="library-scroll">
      <header className="page-heading"><div><span className="eyebrow">YOUR MCP LIBRARY</span><h1>我的 MCP</h1><p className="sub">维护一份配置，按需应用到你的 Agent。</p></div>
        <div className="heading-actions"><button className="btn" onClick={onImport} disabled={blocked}><Icon name="import" />从 Agent 导入</button>
          <button className="btn primary" onClick={onAdd} disabled={blocked || !!error}><Icon name="plus" />添加 MCP</button></div>
      </header>
      {error ? <div className="welcome-state error-state" role="alert"><div className="welcome-symbol"><Icon name="folder" size={28} /></div>
        <h2>暂时无法打开配置库</h2><p>{error}</p><div className="heading-actions"><button className="btn primary" onClick={onManage}>检查配置库位置</button><button className="btn" onClick={onRetry}>重新读取</button></div>
        <small>原文件保持不变。你仍可在 Agent 页面查看和导入已有配置。</small></div>
      : loading && !entries.length ? <div className="welcome-state" role="status"><span className="loading-orbit" /><h2>正在打开你的配置库</h2><p>读取配置和 Agent 状态，不会写入文件。</p></div>
      : !entries.length ? <div className="welcome-state"><div className="welcome-symbol"><Icon name="library" size={32} /></div>
        <span className="eyebrow">START WITH WHAT YOU HAVE</span><h2>把你的第一个 MCP 带进来</h2><p>已经配好 Agent？直接导入。也可以手动添加一个新配置。</p>
        <div className="heading-actions"><button className="btn primary" onClick={onImport} disabled={blocked}><Icon name="import" />从已有 Agent 导入</button>
          <button className="btn" onClick={onAdd} disabled={blocked}><Icon name="plus" />手动添加</button></div>
        <button className="text-button" onClick={onManage}>打开已有配置库</button><small>{missing ? "配置库将在首次保存时创建。" : "当前配置库为空。"}不会自动修改任何 Agent。</small></div>
      : <>
        <div className="library-tools"><label className="search-field"><Icon name="search" /><input aria-label="搜索 MCP" value={query} placeholder="搜索名称、描述或标签" onChange={(event) => setQuery(event.target.value)} /></label>
          <span className="muted">{list.length} 个配置</span><div className="spacer" /><button className="btn ghost small" disabled={!matrix || blocked} onClick={onMatrix}><Icon name="grid" size={16} />批量矩阵</button></div>
        <div className={`library-workspace${entry ? " with-inspector" : ""}`}>
          <div className="library-table-wrap"><table className="library-table"><thead><tr><th>MCP 配置</th><th>适用范围</th><th>当前配置状态</th><th><span className="sr-only">操作</span></th></tr></thead><tbody>
            {list.map((item) => {
              const row = matrix?.rows.find((row) => row.name === item.name);
              const present = row?.cells.filter((cell) => cell.state === "ok" || cell.state === "diff") ?? [];
              const hasDiff = present.some((cell) => cell.state === "diff");
              const reason = scopeReason(item, scope);
              return <tr key={item.name} className={entry?.name === item.name ? "focused" : ""}>
                <td><button className="entry-name" onClick={() => onFocus(item.name)} aria-label={`查看 ${item.name}`} aria-pressed={entry?.name === item.name}>
                  <span className={`entry-symbol ${item.transport === "stdio" ? "local" : "remote"}`}><Icon name={item.transport === "stdio" ? "command" : "globe"} size={20} /></span>
                  <span><b>{item.name}</b><small>{item.description || (item.transport === "stdio" ? "本地命令" : "远程服务")}<span className="meta-divider">·</span>{item.transport}</small></span></button></td>
                <td><span className={`scope-tag${reason ? " restricted" : ""}`}>{scopeLabel(item)}</span></td>
                <td><div className="row-status">{reason ? <span className="muted">需切换范围</span> : !row ? <span className="muted">尚未读取</span> : present.length ? <span className="row-status-line"><span className="agent-stack">{present.slice(0, 5).map((cell) => <AgentBadge key={cell.clientId} clientId={cell.clientId} size="sm" />)}</span><span className={`status-dot${hasDiff ? " warn" : ""}`} />已配置于 {present.length} 个 Agent</span> : <span className="muted">尚未分发</span>}
                  {selectedNames.has(item.name) && <small className="selected-note">已加入本次变更</small>}</div></td>
                <td><button className="btn ghost row-action" aria-label={`分发 ${item.name}`} onClick={() => onFocus(item.name)}>选择目标<Icon name="chevron" size={15} /></button></td>
              </tr>;
            })}
          </tbody></table>{!list.length && <div className="empty-search"><h3>没有匹配的配置</h3><button className="text-button" onClick={() => setQuery("")}>清除搜索</button></div>}
          </div>
          {entry && <aside ref={inspector} className="entry-inspector" aria-label={`${entry.name} 的配置与分发`}>
            <div className="inspector-title"><span className="inspector-kicker">配置详情</span><button className="btn ghost icon-button small" aria-label="收起配置详情" onClick={() => onFocus(null)}><Icon name="close" size={16} /></button></div>
            <h2>{entry.name}</h2><p className="muted inspector-description">{entry.description || "统一管理，按需应用到不同 Agent。"}</p>
            <div className="inspector-meta"><span>{entry.transport === "stdio" ? "本地命令" : "远程服务"}</span><span>{scopeLabel(entry)}</span></div>
            <button className="btn edit-entry" onClick={() => onEdit(entry)} disabled={blocked}><Icon name="edit" size={16} />编辑配置</button>
            <div className="inspector-section"><h3>这次要应用到</h3><p className="field-hint">每个 MCP 的目标独立保存，未勾选的条目不会写入。</p>
              {scopeReason(entry, scope) && <div className="scope-explanation"><p>{scopeReason(entry, scope)}</p>{entry.scope !== "local" && <button className="text-button" onClick={() => onScopeChange(entry.scope as Scope)}>切换到{entry.scope === "project" ? "项目" : "全局"}配置</button>}</div>}
              {visibleClients.map((client) => target(client, entry))}
              {!!otherClients.length && <details className="other-agents"><summary>其他 Agent（{otherClients.length}）</summary><p className="field-hint">未在本机检测到，仍可提前配置。</p><div className="other-agent-list">{otherClients.map((client) => target(client, entry))}</div></details>}
              {!clients && <p className="field-hint">选择有效工作范围后，即可读取 Agent 状态。</p>}
            </div>
            <details className="technical-details"><summary>变量与写入选项</summary><p className="field-hint">变量值和连接状态均未验证。</p>
              <div className="variable-tags">{variableNames(entry).length ? variableNames(entry).map((name) => <code key={name}>{name}</code>) : <span className="muted">没有声明变量引用</span>}</div>
              <label className="field-label">写入策略<select aria-label="写入策略" className="select" value={mode} onChange={(event) => onModeChange(event.target.value as MergeMode)} disabled={blocked}><option value="merge">字段合并 · 保留其他字段</option><option value="replace">整条替换 · 替换选中的同名 MCP</option></select></label>
            </details>
          </aside>}
        </div>
        <p className="library-footnote"><Icon name="shield" size={15} />这里显示配置文件状态，连接状态尚未验证。实际写入路径会在预览中展示。</p>
      </>}
    </div>
    {!!selected.size && <footer className="selection-bar" aria-label="本次分发选择"><span className="selection-mark"><Icon name="check" /></span><div><b>{selectedNames.size} 个 MCP<span className="meta-divider">·</span>{Object.keys(pairs).length} 个 Agent</b><small>{selected.size} 组勾选，确认前可核对逐文件变化</small></div><div className="spacer" />
      <button className="btn primary" onClick={onPlan} disabled={planning || blocked || !matrix}>{planning ? "正在生成预览…" : "预览变更"}<Icon name="arrow" size={16} /></button></footer>}
  </section>;
}
