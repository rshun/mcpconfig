import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import { AgentBadge } from "../components/AgentBadge";
import { Icon } from "../components/Icon";
import { scopeLabel, scopeReason, variableNames } from "../library";
import { selectionFromCells } from "../selection";
import { formatEntryFullCommand, summarizeEntryEndpoint } from "../snippet";
import type { ClientDto, MatrixDto, MergeMode, ProbeDto, Scope } from "../types";
import type { Entry } from "./Catalog";

const CELL = {
  ok: "已配置且一致",
  diff: "已有配置，存在差异",
  absent: "未配置",
  unsupported: "不支持此范围",
};

type StatusFilter = "all" | "deployed" | "undeployed" | "diff";

export function LibraryPage({
  entries,
  matrix,
  clients,
  scope,
  catalogPath,
  projectRoot,
  selected,
  removed,
  focus,
  onFocus,
  onToggle,
  onToggleRemove,
  onSelectDetectedForEntry,
  onClearPending,
  onAdd,
  onEdit,
  onImport,
  onManage,
  onPlan,
  onMatrix,
  onScopeChange,
  mode,
  onModeChange,
  loading,
  planning,
  blocked,
  missing,
  error,
  onRetry,
}: {
  entries: Entry[];
  matrix: MatrixDto | null;
  clients: ClientDto[] | null;
  scope: Scope;
  catalogPath: string;
  projectRoot: string | null;
  selected: Set<string>;
  removed: Set<string>;
  focus: string | null;
  onFocus: (name: string | null) => void;
  onToggle: (key: string) => void;
  onToggleRemove: (key: string) => void;
  onSelectDetectedForEntry: (entryName: string) => void;
  onClearPending: () => void;
  onAdd: () => void;
  onEdit: (entry: Entry) => void;
  onImport: () => void;
  onManage: () => void;
  onPlan: () => void;
  onMatrix: () => void;
  onScopeChange: (scope: Scope) => void;
  mode: MergeMode;
  onModeChange: (mode: MergeMode) => void;
  loading: boolean;
  planning: boolean;
  blocked: boolean;
  missing: boolean;
  error: string | null;
  onRetry: () => void;
}) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<StatusFilter>("all");
  const [copiedName, setCopiedName] = useState<string | null>(null);
  const [probing, setProbing] = useState(false);
  const [probes, setProbes] = useState<Record<string, ProbeDto>>({});
  const inspector = useRef<HTMLElement>(null);

  const list = entries.filter((item) => {
    const hay = `${item.name} ${item.description ?? ""} ${(item.tags ?? []).join(" ")} ${item.command ?? ""} ${item.url ?? ""}`.toLowerCase();
    if (!hay.includes(query.toLowerCase())) return false;
    if (filter === "all") return true;
    const row = matrix?.rows.find((r) => r.name === item.name);
    const present = row?.cells.filter((c) => c.state === "ok" || c.state === "diff") ?? [];
    if (filter === "deployed") return present.length > 0;
    if (filter === "undeployed") return present.length === 0;
    if (filter === "diff") return present.some((c) => c.state === "diff");
    return true;
  });

  const entry = list.find((item) => item.name === focus) ?? entries.find((item) => item.name === focus);
  const writePairs = selectionFromCells(selected);
  const removePairs = selectionFromCells(removed);
  const writeNames = new Set(Object.values(writePairs).flat());
  const removeNames = new Set(Object.values(removePairs).flat());
  const affectedAgents = new Set([...Object.keys(writePairs), ...Object.keys(removePairs)]);
  const totalPendingCount = selected.size + removed.size;

  useEffect(() => {
    if (focus && window.matchMedia("(max-width: 1100px)").matches) {
      inspector.current?.scrollIntoView({ block: "start", behavior: "auto" });
    }
  }, [focus]);

  async function copySpec(targetEntry: Entry) {
    const text = formatEntryFullCommand(targetEntry);
    if (!text || typeof navigator === "undefined" || !navigator.clipboard) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopiedName(targetEntry.name);
      setTimeout(() => setCopiedName((cur) => (cur === targetEntry.name ? null : cur)), 1800);
    } catch {
      // ignore clipboard errors
    }
  }

  async function runProbe(targetEntry: Entry) {
    if (probing) return;
    setProbing(true);
    try {
      const result = await api.probeEntry(catalogPath, targetEntry, scope, projectRoot);
      setProbes((prev) => ({ ...prev, [targetEntry.name]: result }));
    } catch (e) {
      setProbes((prev) => ({
        ...prev,
        [targetEntry.name]: {
          ok: false,
          kind: targetEntry.transport,
          summary: String(e instanceof Error ? e.message : e),
          serverName: null,
          serverVersion: null,
          tools: [],
          missingVars: [],
          durationMs: 0,
        },
      }));
    } finally {
      setProbing(false);
    }
  }

  function target(client: ClientDto, current: Entry) {
    const cell = matrix?.rows
      .find((row) => row.name === current.name)
      ?.cells.find((c) => c.clientId === client.id);
    const disabledByEntry = current.enabled === false && !cell ? "此 MCP 已停用，启用后可分发" : null;
    const reason =
      scopeReason(current, scope) ??
      disabledByEntry ??
      client.scopeBlock ??
      (cell?.state === "unsupported" ? cell.detail || CELL.unsupported : null);
    const key = `${current.name}::${client.id}`;
    const isChosen = selected.has(key);
    const isMarkedRemove = removed.has(key);
    const isInstalled = cell?.state === "ok" || cell?.state === "diff";

    return (
      <label
        className={`agent-target${isChosen ? " chosen" : ""}${isMarkedRemove ? " marked-remove" : ""}${reason ? " unavailable" : ""}`}
        key={client.id}
      >
        <input
          type="checkbox"
          aria-label={`${current.name} → ${client.name}`}
          checked={isChosen}
          disabled={!!reason || !cell || blocked}
          onChange={() => onToggle(key)}
        />
        <AgentBadge clientId={client.id} size="md" />
        <span className="agent-target-copy">
          <b>
            {client.name}
            {isChosen && <span className="pending-tag write">待同步</span>}
            {isMarkedRemove && <span className="pending-tag remove">待移除</span>}
          </b>
          <small>
            {reason ?? (cell ? CELL[cell.state] : "尚未读取此范围的配置")}
          </small>
        </span>
        {isInstalled && !reason && (
          <button
            type="button"
            className={`target-remove-btn${isMarkedRemove ? " active" : ""}`}
            aria-label={`从 ${client.name} 移除 ${current.name}`}
            title={isMarkedRemove ? "取消从该 Agent 移除" : "从该 Agent 移除此 MCP"}
            disabled={blocked}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onToggleRemove(key);
            }}
          >
            {isMarkedRemove ? "撤回" : "移除"}
          </button>
        )}
        {cell?.state === "ok" && !isMarkedRemove && <Icon name="check" size={15} />}
      </label>
    );
  }

  const visibleClients = clients?.filter((client) => client.state !== "missing") ?? [];
  const otherClients = clients?.filter((client) => client.state === "missing") ?? [];
  const currentProbe = entry ? probes[entry.name] : undefined;
  const envStatus = matrix?.envStatus;

  const canBatchSelectDetected =
    !!entry &&
    !scopeReason(entry, scope) &&
    visibleClients.some((client) => {
      const cell = matrix?.rows
        .find((r) => r.name === entry.name)
        ?.cells.find((c) => c.clientId === client.id);
      return !!cell && cell.state !== "unsupported" && !client.scopeBlock;
    });

  return (
    <section className="library-page" aria-label="MCP 配置库">
      <div className="library-scroll">
        <header className="page-heading">
          <div>
            <span className="eyebrow">YOUR MCP LIBRARY</span>
            <h1>我的 MCP</h1>
            <p className="sub">集中维护一份标准配置，按需同步或清理本机各 Agent。</p>
          </div>
          <div className="heading-actions">
            <button className="btn" onClick={onImport} disabled={blocked}>
              <Icon name="import" />从 Agent 导入
            </button>
            <button className="btn primary" onClick={onAdd} disabled={blocked || !!error}>
              <Icon name="plus" />添加 MCP
            </button>
          </div>
        </header>

        {error ? (
          <div className="welcome-state error-state" role="alert">
            <div className="welcome-symbol">
              <Icon name="folder" size={28} />
            </div>
            <h2>暂时无法打开配置库</h2>
            <p>{error}</p>
            <div className="heading-actions">
              <button className="btn primary" onClick={onManage}>
                检查配置库位置
              </button>
              <button className="btn" onClick={onRetry}>
                重新读取
              </button>
            </div>
            <small>原文件保持不变。你仍可在 Agent 页面查看和导入已有配置。</small>
          </div>
        ) : loading && !entries.length ? (
          <div className="welcome-state" role="status">
            <span className="loading-orbit" />
            <h2>正在打开你的配置库</h2>
            <p>读取配置和 Agent 状态，不会写入文件。</p>
          </div>
        ) : !entries.length ? (
          <div className="welcome-state">
            <div className="welcome-symbol">
              <Icon name="library" size={32} />
            </div>
            <span className="eyebrow">START WITH WHAT YOU HAVE</span>
            <h2>把你的第一个 MCP 带进来</h2>
            <p>已经配好 Agent？直接导入。也可以粘贴 JSON / 命令快速新增。</p>
            <div className="heading-actions">
              <button className="btn primary" onClick={onImport} disabled={blocked}>
                <Icon name="import" />从已有 Agent 导入
              </button>
              <button className="btn" onClick={onAdd} disabled={blocked}>
                <Icon name="plus" />手动添加
              </button>
            </div>
            <button className="text-button" onClick={onManage}>
              打开已有配置库
            </button>
            <small>
              {missing ? "配置库将在首次保存时创建。" : "当前配置库为空。"}不会自动修改任何 Agent。
            </small>
          </div>
        ) : (
          <>
            <div className="library-tools">
              <label className="search-field">
                <Icon name="search" />
                <input
                  aria-label="搜索 MCP"
                  value={query}
                  placeholder="搜索名称、命令、URL 或标签"
                  onChange={(event) => setQuery(event.target.value)}
                />
              </label>
              <div className="segmented-filters" role="group" aria-label="按分发状态筛选">
                {(
                  [
                    ["all", "全部"],
                    ["deployed", "已分发"],
                    ["undeployed", "未分发"],
                    ["diff", "有差异"],
                  ] as const
                ).map(([id, label]) => (
                  <button
                    key={id}
                    type="button"
                    className={`seg-btn${filter === id ? " active" : ""}`}
                    aria-pressed={filter === id}
                    onClick={() => setFilter(id)}
                  >
                    {label}
                  </button>
                ))}
              </div>
              <span className="muted count-summary">{list.length} 项</span>
              <div className="spacer" />
              <button
                className="btn small matrix-trigger"
                disabled={!matrix || blocked}
                onClick={onMatrix}
              >
                <Icon name="grid" size={15} />
                批量矩阵
              </button>
            </div>

            <div className={`library-workspace${entry ? " with-inspector" : ""}`}>
              <div className="library-table-wrap">
                <table className="library-table">
                  <thead>
                    <tr>
                      <th>MCP 配置</th>
                      <th>适用范围</th>
                      <th>当前配置状态</th>
                      <th>
                        <span className="sr-only">操作</span>
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {list.map((item) => {
                      const row = matrix?.rows.find((r) => r.name === item.name);
                      const present =
                        row?.cells.filter((c) => c.state === "ok" || c.state === "diff") ?? [];
                      const hasDiff = present.some((c) => c.state === "diff");
                      const reason = scopeReason(item, scope);
                      const isPaused = item.enabled === false;
                      const pendingWrite = writeNames.has(item.name);
                      const pendingRem = removeNames.has(item.name);

                      return (
                        <tr
                          key={item.name}
                          className={`${entry?.name === item.name ? "focused" : ""}${isPaused ? " paused-row" : ""}`}
                        >
                          <td>
                            <button
                              className="entry-name"
                              onClick={() => onFocus(item.name)}
                              aria-label={`查看 ${item.name}`}
                              aria-pressed={entry?.name === item.name}
                            >
                              <span
                                className={`entry-symbol ${item.transport === "stdio" ? "local" : "remote"}`}
                              >
                                <Icon
                                  name={item.transport === "stdio" ? "command" : "globe"}
                                  size={20}
                                />
                              </span>
                              <span className="entry-main-copy">
                                <span className="entry-title-row">
                                  <b>{item.name}</b>
                                  <span className="proto-pill">{item.transport}</span>
                                  {isPaused && <span className="paused-pill">已停用</span>}
                                </span>
                                <small>
                                  {item.description ||
                                    (item.transport === "stdio" ? "本地进程服务" : "远程 MCP 服务")}
                                </small>
                                <code className="endpoint-preview">
                                  {summarizeEntryEndpoint(item)}
                                </code>
                              </span>
                            </button>
                          </td>
                          <td>
                            <span className={`scope-tag${reason ? " restricted" : ""}`}>
                              {scopeLabel(item)}
                            </span>
                          </td>
                          <td>
                            <div className="row-status">
                              {reason ? (
                                <span className="muted">需切换范围</span>
                              ) : isPaused && !row ? (
                                <span className="muted">已停用 · 暂不分发</span>
                              ) : !row ? (
                                <span className="muted">尚未读取</span>
                              ) : present.length ? (
                                <span className="row-status-line">
                                  <span className="agent-stack">
                                    {present.slice(0, 5).map((cell) => (
                                      <AgentBadge
                                        key={cell.clientId}
                                        clientId={cell.clientId}
                                        size="sm"
                                      />
                                    ))}
                                  </span>
                                  <span className={`status-dot${hasDiff ? " warn" : ""}`} />
                                  已配置于 {present.length} 个 Agent
                                </span>
                              ) : (
                                <span className="muted">尚未分发</span>
                              )}
                              {(pendingWrite || pendingRem) && (
                                <small className={`selected-note${pendingRem && !pendingWrite ? " danger-note" : ""}`}>
                                  {pendingWrite && pendingRem
                                    ? "含待同步与待移除目标"
                                    : pendingRem
                                      ? "已标记待移除"
                                      : "已加入本次变更"}
                                </small>
                              )}
                            </div>
                          </td>
                          <td>
                            <button
                              className="btn ghost row-action"
                              aria-label={`分发 ${item.name}`}
                              onClick={() => onFocus(item.name)}
                            >
                              选择目标
                              <Icon name="chevron" size={15} />
                            </button>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
                {!list.length && (
                  <div className="empty-search">
                    <h3>没有匹配的配置</h3>
                    <button
                      className="text-button"
                      onClick={() => {
                        setQuery("");
                        setFilter("all");
                      }}
                    >
                      重置筛选条件
                    </button>
                  </div>
                )}
              </div>

              {entry && (
                <aside
                  ref={inspector}
                  className="entry-inspector"
                  aria-label={`${entry.name} 的配置与分发`}
                >
                  <div className="inspector-title">
                    <span className="inspector-kicker">配置详情</span>
                    <button
                      className="btn ghost icon-button small"
                      aria-label="收起配置详情"
                      onClick={() => onFocus(null)}
                    >
                      <Icon name="close" size={16} />
                    </button>
                  </div>
                  <h2>{entry.name}</h2>
                  <p className="muted inspector-description">
                    {entry.description || "统一管理，按需应用到不同 Agent。"}
                  </p>
                  <div className="inspector-meta">
                    <span>{entry.transport === "stdio" ? "本地命令 · stdio" : `远程 · ${entry.transport.toUpperCase()}`}</span>
                    <span>{scopeLabel(entry)}</span>
                    {entry.enabled === false && <span className="warn-chip">已停用</span>}
                    {entry.timeout_seconds && <span>超时 {entry.timeout_seconds}s</span>}
                    {(entry.tags ?? []).map((t) => (
                      <span key={t}>#{t}</span>
                    ))}
                  </div>

                  <div className="inspector-spec-card">
                    <div className="spec-card-top">
                      <span>{entry.transport === "stdio" ? "启动命令" : "服务地址"}</span>
                      {typeof navigator !== "undefined" && !!navigator.clipboard && (
                        <button
                          type="button"
                          className="text-button spec-copy-btn"
                          onClick={() => copySpec(entry)}
                        >
                          <Icon name={copiedName === entry.name ? "check" : "copy"} size={13} />
                          {copiedName === entry.name ? "已复制" : "复制"}
                        </button>
                      )}
                    </div>
                    <code className="spec-command-code">{formatEntryFullCommand(entry) || "—"}</code>
                  </div>

                  <div className="inspector-primary-actions">
                    <button
                      className="btn edit-entry"
                      onClick={() => onEdit(entry)}
                      disabled={blocked}
                    >
                      <Icon name="edit" size={15} />
                      编辑配置
                    </button>
                    <button
                      type="button"
                      className="btn probe-btn"
                      onClick={() => runProbe(entry)}
                      disabled={probing || blocked}
                      title={
                        entry.transport === "stdio"
                          ? "拉起受控子进程执行 JSON-RPC initialize 握手并读取工具列表"
                          : "校验远程服务 URL 与认证环境变量是否就绪"
                      }
                    >
                      <Icon name="play" size={14} />
                      {probing ? "检测中…" : entry.transport === "stdio" ? "测试启动" : "校验配置"}
                    </button>
                  </div>

                  {currentProbe && (
                    <div
                      className={`probe-result-card ${currentProbe.ok ? "ok" : "fail"}`}
                      role="status"
                    >
                      <div className="probe-result-head">
                        <span className={`status-dot${currentProbe.ok ? "" : " warn"}`} />
                        <b>{currentProbe.summary}</b>
                      </div>
                      {currentProbe.tools.length > 0 && (
                        <div className="probe-tools">
                          {currentProbe.tools.slice(0, 10).map((tool) => (
                            <code key={tool}>{tool}</code>
                          ))}
                          {currentProbe.tools.length > 10 && (
                            <small>+{currentProbe.tools.length - 10} 个工具</small>
                          )}
                        </div>
                      )}
                    </div>
                  )}

                  <div className="inspector-section">
                    <div className="inspector-section-head">
                      <h3>目标 Agent</h3>
                      {canBatchSelectDetected && (
                        <button
                          type="button"
                          className="text-button"
                          disabled={blocked}
                          onClick={() => onSelectDetectedForEntry(entry.name)}
                        >
                          全选本机 Agent
                        </button>
                      )}
                    </div>
                    <p className="field-hint">
                      勾选需要写入的目标，或将已安装的 Agent 标记为移除；确认前可逐文件预览差异。
                    </p>
                    {scopeReason(entry, scope) && (
                      <div className="scope-explanation">
                        <p>{scopeReason(entry, scope)}</p>
                        <button
                          className="text-button"
                          onClick={() => onScopeChange(entry.scope as Scope)}
                        >
                          切换到
                          {entry.scope === "project"
                            ? "项目"
                            : entry.scope === "local"
                              ? "本地"
                              : "全局"}
                          配置
                        </button>
                      </div>
                    )}
                    {visibleClients.map((client) => target(client, entry))}
                    {!!otherClients.length && (
                      <details className="other-agents">
                        <summary>其他 Agent（{otherClients.length}）</summary>
                        <p className="field-hint">未在本机检测到，仍可提前生成配置文件。</p>
                        <div className="other-agent-list">
                          {otherClients.map((client) => target(client, entry))}
                        </div>
                      </details>
                    )}
                    {!clients && (
                      <p className="field-hint">选择有效工作范围后，即可读取 Agent 状态。</p>
                    )}
                  </div>

                  <details className="technical-details">
                    <summary>环境变量与写入策略</summary>
                    <p className="field-hint">
                      实时检测当前宿主环境变量是否已设置（仅判断存在性，不读取明文）。
                    </p>
                    <div className="variable-tags">
                      {variableNames(entry).length ? (
                        variableNames(entry).map((name) => {
                          const readyState = envStatus ? envStatus[name] : undefined;
                          return (
                            <code
                              key={name}
                              className={
                                readyState === false
                                  ? "var-missing"
                                  : readyState === true
                                    ? "var-ready"
                                    : ""
                              }
                              title={
                                readyState === true
                                  ? `本机环境变量 ${name} 已设置`
                                  : readyState === false
                                    ? `本机尚未检测到环境变量 ${name}`
                                    : name
                              }
                            >
                              {readyState !== undefined && (
                                <span
                                  className={`var-dot ${readyState ? "ok" : "warn"}`}
                                  aria-hidden="true"
                                />
                              )}
                              {name}
                              {readyState === false && <em>未设置</em>}
                            </code>
                          );
                        })
                      ) : (
                        <span className="muted">没有声明环境变量引用</span>
                      )}
                    </div>
                    <label className="field-label">
                      写入策略
                      <select
                        aria-label="写入策略"
                        className="select"
                        value={mode}
                        onChange={(event) =>
                          onModeChange(event.target.value as MergeMode)
                        }
                        disabled={blocked}
                      >
                        <option value="merge">字段合并 · 保留其他字段</option>
                        <option value="replace">整条替换 · 替换选中的同名 MCP</option>
                      </select>
                    </label>
                  </details>
                </aside>
              )}
            </div>

            <p className="library-footnote">
              <Icon name="shield" size={15} />
              任何写入或移除操作均会先展示逐文件差异预览，并在落盘前自动备份原文件。
            </p>
          </>
        )}
      </div>

      {totalPendingCount > 0 && (
        <footer className="selection-bar" aria-label="本次分发选择">
          <span className="selection-mark">
            <Icon name="check" />
          </span>
          <div>
            <b>
              {selected.size > 0 && `${writeNames.size} 个 MCP 待同步`}
              {selected.size > 0 && removed.size > 0 && <span className="meta-divider">·</span>}
              {removed.size > 0 && `${removed.size} 处配置待移除`}
              <span className="meta-divider">·</span>
              涉及 {affectedAgents.size} 个 Agent
            </b>
            <small>
              共 {totalPendingCount} 项待处理变更，点击右侧预览逐文件改动后再确认落盘
            </small>
          </div>
          <div className="spacer" />
          <button
            type="button"
            className="btn ghost small"
            disabled={planning || blocked}
            onClick={onClearPending}
          >
            清空重选
          </button>
          <button
            className="btn primary"
            onClick={onPlan}
            disabled={planning || blocked || (!matrix && selected.size > 0)}
          >
            {planning ? "正在生成预览…" : "预览变更"}
            <Icon name="arrow" size={16} />
          </button>
        </footer>
      )}
    </section>
  );
}
