import { useState } from "react";
import type { ClientDto, MatrixDto } from "../types";
import type { Entry } from "./Catalog";
import { Empty } from "../components/ui";
import { selectionFromCells } from "../selection";

const STATUS = { ok: "已配置且一致", absent: "未配置", diff: "有差异", unsupported: "不支持" };

export function Workbench({ matrix, entries, clients, selected, onToggle, onPlan, onEdit, loading }: {
  matrix: MatrixDto | null; entries: Entry[]; clients: ClientDto[] | null; selected: Set<string>;
  onToggle: (key: string) => void; onPlan: () => void; onEdit: () => void; loading: boolean;
}) {
  const [view, setView] = useState<"mcp" | "agent">("mcp");
  const [focus, setFocus] = useState("");
  const [agent, setAgent] = useState("");
  const [query, setQuery] = useState("");
  const rows = matrix?.rows ?? [];
  const row = rows.find((r) => r.name === focus) ?? rows[0];
  const target = matrix?.clients.find((c) => c.id === agent) ?? matrix?.clients[0];
  const entry = entries.find((e) => e.name === row?.name);
  const pairs = selectionFromCells(selected);
  const chosen = new Set(Object.values(pairs).flat()).size;
  const agents = Object.keys(pairs).length;
  return <>
    <div className="page-heading"><div><div className="eyebrow">MCP DISTRIBUTION</div><h1>配置工作台</h1>
      <p className="sub">选择一个 MCP，为它勾选目标 Agent。预览后再应用，未勾选的配置不会写入。</p></div>
      <button className="btn" onClick={onEdit}>整理配置库</button></div>
    <div className="work-toolbar"><div className="segments">
      <button className={view === "mcp" ? "active" : ""} onClick={() => setView("mcp")}>按 MCP 配置</button>
      <button className={view === "agent" ? "active" : ""} onClick={() => setView("agent")}>按 Agent 查看</button>
    </div><span className="muted">{rows.length} 个 MCP · {matrix?.clients.length ?? 0} 个 Agent</span></div>
    {!matrix || !row ? <Empty title={loading ? "正在读取配置…" : "当前范围没有可分发的 MCP"}
      hint="先加载或新建配置库，再添加跟随当前范围的 MCP。Agent 页面可以独立查看已有配置。" /> : <div className="workbench">
      <section className="work-list" aria-label={view === "mcp" ? "MCP 列表" : "Agent 列表"}>
        <input className="input" aria-label="搜索配置或 Agent" placeholder="搜索配置或 Agent…" value={query} onChange={(e) => setQuery(e.target.value)} />
        {view === "mcp" ? rows.filter((r) => r.name.toLowerCase().includes(query.toLowerCase())).map((r) => <button
          key={r.name} className={`work-item${row.name === r.name ? " active" : ""}`} onClick={() => setFocus(r.name)}>
          <span className="work-icon">{r.transport === "stdio" ? "⌘" : "↗"}</span><span><b>{r.name}</b><small>{r.transport} · {r.cells.filter((c) => selected.has(`${r.name}::${c.clientId}`)).length} 个目标已选</small></span>
        </button>) : matrix.clients.filter((c) => c.name.toLowerCase().includes(query.toLowerCase())).map((c) => <button key={c.id}
          className={`work-item${target?.id === c.id ? " active" : ""}`} onClick={() => setAgent(c.id)}><span className="work-icon">A</span><span><b>{c.name}</b>
          <small>{c.kind} · {rows.filter((r) => selected.has(`${r.name}::${c.id}`)).length} 个 MCP 已选</small></span></button>)}
      </section>
      <section className="work-detail">
        <div className="eyebrow">{view === "mcp" ? "CONFIGURATION" : "TARGET AGENT"}</div>
        <h2>{view === "mcp" ? row.name : target?.name}</h2>
        <p className="muted">{view === "mcp" ? entry?.description || "从一份配置库分发到多个 Agent" : "只为当前 Agent 勾选需要写入的 MCP。每组勾选独立保存。"}</p>
        {view === "mcp" && <><div className="detail-facts"><span>连接方式<b>{row.transport}</b></span><span>配置范围<b>{row.scope === "user" ? "全局配置" : "项目配置"}</b></span></div>
          <div className="detail-note"><b>分发流程</b><p>1. 在右侧勾选目标 Agent</p><p>2. 预览逐文件差异与路径</p><p>3. 确认写入，自动保留原文备份</p></div>
          <p className="muted">地址、参数及凭据请在配置库编辑器中核对；工作台不展开敏感内容。</p></>}
        {view === "agent" && <div className="target-list">{rows.map((r) => {
          const cell = r.cells.find((c) => c.clientId === target?.id);
          if (!cell || !target) return null;
          const key = `${r.name}::${target.id}`;
          return <label className="target" key={r.name}><input type="checkbox" disabled={cell.state === "unsupported"} checked={selected.has(key)} onChange={() => onToggle(key)}
            aria-label={`${r.name} → ${target.name}`} /><span><b>{r.name}</b><small>{STATUS[cell.state]}</small></span></label>;
        })}</div>}
      </section>
      <section className="work-targets"><div className="section-label">{view === "mcp" ? "目标 Agent" : "当前目标路径"}</div>
        {view === "mcp" ? row.cells.map((cell) => {
          const c = matrix.clients.find((client) => client.id === cell.clientId)!;
          const detected = clients?.find((client) => client.id === c.id);
          const key = `${row.name}::${c.id}`;
          return <label key={c.id} className={`target${selected.has(key) ? " selected" : ""}`} title={cell.detail ?? detected?.path ?? ""}>
            <input type="checkbox" disabled={cell.state === "unsupported"} checked={selected.has(key)} onChange={() => onToggle(key)} aria-label={`${row.name} → ${c.name}`} />
            <span><b>{c.name}</b><small>{cell.state === "unsupported" ? cell.detail || "不支持此范围" : `${STATUS[cell.state]} · ${detected?.stateLabel ?? c.kind}`}</small></span>
          </label>;
        }) : <p className="mono path-note">{clients?.find((c) => c.id === target?.id)?.path ?? "当前范围没有可用路径"}</p>}
        <p className="muted target-hint">全局配置也只应用到已勾选的 Agent。多个 Agent 共用同一文件时，预览会明确提示。</p>
      </section>
    </div>}
    <div className="bottombar"><span><b>{chosen}</b> 个 MCP · <b>{agents}</b> 个 Agent · <b>{selected.size}</b> 组勾选</span>
      <span className="hint">写入前自动备份</span><div className="spacer" /><button className="btn primary" disabled={!selected.size || loading} onClick={onPlan}>预览变更 →</button></div>
  </>;
}
