import type { FileDto, PlanDto } from "../types";
import { Banner, Empty, Pill } from "../components/ui";

function DiffBody({ file }: { file: FileDto }) {
  if (!file.diff) {
    return (
      <div className="diff-body">
        <div className="center-box" style={{ padding: 20 }}>
          {file.existed ? "内容已一致，无需写入" : "将新建此文件"}
        </div>
      </div>
    );
  }
  return (
    <div className="diff-body">
      {file.diff.split("\n").map((line, i) => {
        const sign = line[0];
        const kind = sign === "+" ? "add" : sign === "-" ? "del" : "ctx";
        return (
          <div key={i} className={`diff-line ${kind}`}>
            <span className="sign">{kind === "ctx" ? "" : sign}</span>
            <span>{kind === "ctx" ? line : line.slice(1)}</span>
          </div>
        );
      })}
    </div>
  );
}

export function DiffPage({
  plan,
  applying,
  onApply,
}: {
  plan: PlanDto | null;
  applying: boolean;
  onApply: () => void;
}) {
  if (!plan) {
    return (
      <Empty
        title="还没有生成计划"
        hint="回到「客户端配置」勾选目标，然后点「生成差异预览」"
      />
    );
  }

  const blocked = plan.files.filter((f) => f.blocked);
  const writable = plan.files.filter((f) => f.willWrite);
  const unchanged = plan.files.filter((f) => !f.blocked && !f.willWrite);

  return (
    <>
      {plan.missingVars.length > 0 && (
        <Banner tone="warn">
          <div>
            <b>未解析的环境变量</b>（写入时会保留占位符）：
            <div className="mono">{plan.missingVars.join("、")}</div>
          </div>
        </Banner>
      )}
      {plan.skipped.length > 0 && (
        <Banner tone="warn">
          <div>
            <b>已跳过 {plan.skipped.length} 个客户端</b>
            {plan.skipped.map((s) => (
              <div key={s.clientId} className="mono">
                · {s.clientId}：{s.reason}
              </div>
            ))}
          </div>
        </Banner>
      )}

      <div style={{ display: "flex", alignItems: "baseline", gap: 10, marginBottom: 12 }}>
        <h1 style={{ margin: 0 }}>差异预览</h1>
        <span className="muted">
          {plan.entryCount} 条配置 · {writable.length} 个文件待写入
          {unchanged.length > 0 && ` · ${unchanged.length} 个无变化`}
          {blocked.length > 0 && ` · ${blocked.length} 个被跳过`}
        </span>
        <div className="spacer" />
        <button
          className="btn primary"
          disabled={applying || writable.length === 0}
          onClick={onApply}
        >
          {applying ? "写入中…" : `确认写入 ${writable.length} 个文件`}
        </button>
      </div>

      {writable.length === 0 && (
        <Banner tone="ok">
          所有目标内容均已一致，无需写入（幂等）。
        </Banner>
      )}

      {plan.files.map((file) => {
        if (file.blocked) {
          return (
            <div className="diff-file" key={file.clientId}>
              <div className="diff-head">
                <b>{file.clientName}</b>
                <Pill tone="warn">已跳过</Pill>
                <span className="mono muted">{file.path}</span>
              </div>
              <div className="diff-body">
                <div className="center-box" style={{ padding: 18 }}>
                  {file.blocked}
                </div>
              </div>
            </div>
          );
        }
        if (!file.willWrite) return null;
        return (
          <div className="diff-file" key={file.clientId}>
            <div className="diff-head">
              <b>{file.clientName}</b>
              <Pill>{file.format}</Pill>
              {file.newCount > 0 && (
                <Pill tone="ok">
                  新增 {file.newCount}
                </Pill>
              )}
              {file.updateCount > 0 && (
                <Pill tone="warn">
                  修改 {file.updateCount}
                </Pill>
              )}
              <span className="mono muted">{file.path}</span>
              <div className="spacer" />
              <span className="muted">
                {file.existed ? "已存在，将合并" : "将新建"}
              </span>
            </div>
            <DiffBody file={file} />
            {file.warnings.map((w) => (
              <div className="banner warn" style={{ margin: 0, borderRadius: 0 }} key={w}>
                {w}
              </div>
            ))}
          </div>
        );
      })}
    </>
  );
}
