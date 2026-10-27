import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { BackupDto, ClientDto, ExistingConfigDto, ReceiptDto, Scope } from "../types";
import { Banner, Empty, Pill } from "../components/ui";

const STATE_TONE: Record<string, "ok" | "warn" | "info" | "danger" | "plain"> = {
  ready: "ok",
  installed: "info",
  configOnly: "warn",
  missing: "plain",
};

export function DetectPage({ clients, scope }: { clients: ClientDto[] | null; scope: Scope }) {
  const [config, setConfig] = useState<ExistingConfigDto | null>(null);
  const [reading, setReading] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  useEffect(() => {
    revision.current += 1;
    setConfig(null);
    setReading(null);
    setError(null);
    return () => { revision.current += 1; };
  }, [clients]);

  async function readConfig(clientId: string) {
    const request = ++revision.current;
    setReading(clientId);
    setConfig(null);
    setError(null);
    try {
      const result = await api.readClientConfig(clientId, scope);
      if (request === revision.current) setConfig(result);
    } catch (e) {
      if (request === revision.current) setError(String(e));
    } finally {
      if (request === revision.current) setReading(null);
    }
  }

  if (!clients) return <Empty title="正在检测…" />;

  const ready = clients.filter((c) => c.state === "ready").length;

  return (
    <>
      <h1>环境检测</h1>
      <p className="sub">
        三通道探测：配置路径（最权威）→ CLI 可执行文件 → 应用安装目录。共 {clients.length} 个客户端，
        {ready} 个已就绪。
      </p>

      <div className="panel" style={{ marginBottom: 16 }}>
        <table className="grid">
          <thead>
            <tr>
              <th>客户端</th>
              <th>类型</th>
              <th>状态</th>
              <th>已配置</th>
              <th>可执行文件</th>
              <th>配置路径</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {clients.map((c) => (
              <tr key={c.id}>
                <td>
                  <b>{c.name}</b>
                  <div className="muted" style={{ fontSize: 11 }}>
                    {c.id}
                  </div>
                </td>
                <td>{c.kind}</td>
                <td>
                  {c.scopeBlock ? (
                    <Pill tone="danger">不支持当前作用域</Pill>
                  ) : (
                    <Pill tone={STATE_TONE[c.state] ?? "plain"}>{c.stateLabel}</Pill>
                  )}
                </td>
                <td className="mono">{c.configured}</td>
                <td className="mono muted" style={{ maxWidth: 260, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={c.binary ?? ""}>
                  {c.binary ?? "—"}
                </td>
                <td className="mono muted" style={{ maxWidth: 320, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={c.path ?? ""}>
                  {c.path ?? "—"}
                </td>
                <td>
                  <button className="btn small" disabled={!!c.scopeBlock}
                    title={c.scopeBlock ?? `查看 ${c.name} 的已有 MCP`}
                    onClick={() => readConfig(c.id)}>
                    {reading === c.id ? "读取中…" : "查看 MCP"}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {error && <Banner tone="danger">{error}</Banner>}
      {config && (
        <section className="panel" aria-label="已有 MCP 配置" style={{ marginBottom: 16 }}>
          <div className="panel-head">
            <b>{config.clientName} · 已有 MCP（{config.entries.length}）</b>
            <span className="pill">{config.format}</span>
          </div>
          <div className="panel-body">
            <div className="mono" style={{ overflowWrap: "anywhere", marginBottom: 12 }}>{config.path}</div>
            {!config.existed ? <Empty title="配置文件尚不存在" /> : config.entries.length === 0 ? (
              <Empty title="配置文件中没有 MCP 条目" />
            ) : config.entries.map((entry) => (
              <details key={entry.name} style={{ marginBottom: 10 }}>
                <summary>{entry.name}</summary>
                <pre className="mono" style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>
                  {JSON.stringify(entry.config, null, 2)}
                </pre>
              </details>
            ))}
            <p className="muted">只读查看。密钥、环境变量值、参数和地址等已隐藏；不会修改配置或导入清单。</p>
          </div>
        </section>
      )}

      <Banner>
        未安装的客户端也允许写入（为 Bundle 分发与新机器预配置留路）。检测只回答「谁、在哪」，
        不会擅自把现有配置合并进清单。
      </Banner>
    </>
  );
}

export function BackupsPage({
  backups,
  onRestore,
  receipt,
}: {
  backups: BackupDto[] | null;
  onRestore: (id: string) => void;
  receipt: ReceiptDto | null;
}) {
  if (!backups) return <Empty title="正在读取备份…" />;

  return (
    <>
      <h1>备份与回滚</h1>
      <p className="sub">
        每次写入前都会完整备份原文，可回滚到任意时间点。备份目录权限仅当前用户可读。
      </p>

      {receipt && (
        <Banner tone={receipt.failed > 0 ? "danger" : "ok"}>
          <div>
            <b>上次写入</b>：{receipt.written} 个文件
            {receipt.blocked > 0 && ` · 跳过 ${receipt.blocked}`}
            {receipt.failed > 0 && ` · 失败 ${receipt.failed}`}
            {receipt.backupId && (
              <span className="mono"> · 备份 {receipt.backupId}</span>
            )}
            {receipt.outcomes
              .filter((o) => o.status === "failed")
              .map((o) => (
                <div key={o.clientId} className="mono">
                  · {o.clientId}：{o.message}
                </div>
              ))}
          </div>
        </Banner>
      )}

      {backups.length === 0 ? (
        <Empty title="还没有备份" hint="执行一次写入后会自动生成" />
      ) : (
        <div className="panel">
          <table className="grid">
            <thead>
              <tr>
                <th>备份 ID</th>
                <th>时间</th>
                <th>文件数</th>
                <th>涉及文件</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {backups.map((b) => (
                <tr key={b.id}>
                  <td className="mono">{b.id}</td>
                  <td className="mono muted">{b.createdAt}</td>
                  <td>{b.files}</td>
                  <td className="mono muted" style={{ fontSize: 11 }}>
                    {b.paths.slice(0, 3).join(" · ")}
                    {b.paths.length > 3 && ` …`}
                  </td>
                  <td style={{ textAlign: "right" }}>
                    <button className="btn small" onClick={() => onRestore(b.id)}>
                      回滚
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}
