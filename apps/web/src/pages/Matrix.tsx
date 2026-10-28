import type { MatrixDto } from "../types";
import { Empty } from "../components/ui";
import { selectionFromCells } from "../selection";

const SCOPE_LABEL: Record<string, string> = {
  user: "用户级",
  project: "项目级",
  local: "本地级",
};

interface Props {
  matrix: MatrixDto | null;
  selected: Set<string>;
  onToggle: (key: string) => void;
  onSelectAllClient: (clientId: string) => void;
  onPlan: () => void;
  loading: boolean;
}

export function MatrixPage({
  matrix,
  selected,
  onToggle,
  onSelectAllClient,
  onPlan,
  loading,
}: Props) {
  if (!matrix) {
    return <Empty title="正在读取清单…" hint="若长时间无响应，请检查清单路径" />;
  }
  if (matrix.rows.length === 0) {
    return (
      <Empty
        title="当前作用域下没有配置项"
        hint={`清单里没有 ${SCOPE_LABEL[matrix.scope] ?? matrix.scope} 的条目。可在「MCP配置」页新增，或切换顶部作用域。`}
      />
    );
  }

  const total = selected.size;
  const pairs = selectionFromCells(selected);
  const rowCount = new Set(Object.values(pairs).flat()).size;
  const colCount = Object.keys(pairs).length;

  return (
    <>
      {matrix.issues.length > 0 && (
        <div className="banner warn">
          <div>
            {matrix.issues.map((i) => (
              <div key={i}>{i}</div>
            ))}
          </div>
        </div>
      )}

      <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
        <div className="matrix-wrap">
          <table className="matrix">
            <thead>
              <tr>
                <th className="rowhead">MCP 配置</th>
                {matrix.clients.map((c) => (
                  <th key={c.id} style={{ minWidth: 84 }}>
                    <button
                      className="btn ghost small"
                      title={`全选 ${c.name} 这一列`}
                      onClick={() => onSelectAllClient(c.id)}
                      style={{ padding: 0, border: 0, fontSize: 11, fontWeight: 500 }}
                    >
                      {c.name}
                    </button>
                    <div style={{ fontWeight: 400, color: "var(--text-3)", fontSize: 10 }}>
                      {c.kind}
                    </div>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {matrix.rows.map((row) => (
                <tr key={row.name}>
                  <th className="rowhead">
                    <div className="name">
                      {row.name}
                      <span className="pill plain">
                        {SCOPE_LABEL[row.scope] ?? row.scope}
                      </span>
                    </div>
                    <div className="meta">
                      {row.transport === "stdio" ? "本地 · stdio" : `远程 · ${row.transport}`}
                    </div>
                  </th>
                  {row.cells.map((cell) => {
                    const key = `${row.name}::${cell.clientId}`;
                    const isSel = selected.has(key);
                    const unsupported = cell.state === "unsupported";
                    return (
                      <td key={cell.clientId}>
                        <button
                          className={`cell-btn${isSel ? " sel" : ""}`}
                          disabled={unsupported}
                          title={
                            unsupported
                              ? cell.detail ?? "不支持"
                              : `${row.name} → ${matrix.clients.find((c) => c.id === cell.clientId)?.name}`
                          }
                          onClick={() => onToggle(key)}
                        >
                          <span className={`dot ${cell.state}`} />
                          <span className="tick">{isSel ? "✓" : ""}</span>
                        </button>
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <div className="bottombar">
          <span className="count">
            已选 {rowCount} 个配置 · {colCount} 个客户端 · {total} 组勾选
          </span>
          <span className="hint">写入前会展示逐文件 diff</span>
          <div className="spacer" />
          <button className="btn primary" disabled={total === 0 || loading} onClick={onPlan}>
            生成差异预览
          </button>
        </div>
      </div>

      <div className="legend">
        <span>
          <i className="dot ok" />
          已配置且一致
        </span>
        <span>
          <i className="dot absent" />
          未配置
        </span>
        <span>
          <i className="dot diff" />
          已配置但有差异
        </span>
        <span>
          <i className="dot unsupported" />
          该客户端不支持此作用域
        </span>
        <span className="muted">点列头可整列全选；未勾选不等于删除</span>
      </div>
    </>
  );
}
