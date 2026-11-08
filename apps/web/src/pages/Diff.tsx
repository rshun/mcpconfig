import { AgentBadge } from "../components/AgentBadge";
import { Icon } from "../components/Icon";
import { Banner, Pill } from "../components/ui";
import type { FileDto, MergeMode, PlanDto } from "../types";

function DiffBody({ file }: { file: FileDto }) {
  return <pre className="diff-body">{file.diff.split("\n").map((line, index) => <div key={index} className={`diff-line ${line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx"}`}>{line || " "}</div>)}</pre>;
}
export function DiffPage({ plan, error, applying, onApply, onBack, mode, onModeChange, onReplan, planning }: {
  error?: string | null;
  plan: PlanDto | null; applying: boolean; onApply: () => void; onBack: () => void; mode: MergeMode;
  onModeChange: (mode: MergeMode) => void; onReplan: () => void; planning: boolean;
}) {
  const writable = plan?.files.filter((file) => file.willWrite && !file.blocked) ?? [];
  const blocked = plan?.files.filter((file) => file.blocked) ?? [];
  return <section className="preview-page" aria-label="分发变更预览">
    {error && <Banner tone="danger">{error}</Banner>}
    <div className="preview-intro"><span className="preview-symbol"><Icon name="shield" size={23} /></span><div><h3>先核对影响，再确认写入</h3><p>只处理勾选的配置，其他条目保持不变。实际写入前会备份原文。</p></div></div>
    <details className="preview-options"><summary>写入策略：{mode === "merge" ? "字段合并" : "整条替换"}</summary><label className="field-label">处理同名 MCP<select className="select" aria-label="预览写入策略" value={mode} disabled={applying || planning} onChange={(event) => onModeChange(event.target.value as MergeMode)}><option value="merge">字段合并 · 保留其他字段</option><option value="replace">整条替换 · 未选中的条目保留</option></select></label><p className="field-hint">策略变更后需要重新生成预览。</p></details>
    {!plan ? <div className="welcome-state compact"><h3>需要重新预览</h3><p>选择或写入策略已变化，旧预览不能用于确认。</p><button className="btn primary" disabled={planning} onClick={onReplan}>{planning ? "正在生成…" : "重新生成预览"}</button></div> : <>
      <div className="preview-counts"><b>{plan.entryCount} 个 MCP</b><span>涉及 {plan.files.length} 个实际文件</span><span className={blocked.length ? "warning-text" : ""}>{writable.length} 个待写入{blocked.length ? ` · ${blocked.length} 个被阻止` : ""}</span></div>
      {!!plan.issues.length && <Banner tone="warn"><div>{plan.issues.map((issue) => <p key={issue}>{issue}</p>)}</div></Banner>}
      {!!plan.missingVars.length && <Banner tone="warn"><div><b>存在未解析的变量</b><p>写入会保留占位符，请核对目标运行环境：<span className="mono">{plan.missingVars.join("、")}</span></p></div></Banner>}
      {!!plan.skipped.length && <Banner tone="warn"><div><b>已跳过 {plan.skipped.length} 个 Agent</b>{plan.skipped.map((item) => <p key={item.clientId}>{item.clientId}：{item.reason}</p>)}</div></Banner>}
      {!writable.length && <Banner tone={blocked.length || plan.skipped.length ? "warn" : "ok"}>{blocked.length || plan.skipped.length ? "本次没有可写入的目标，请检查阻止和跳过原因。" : "目标内容已一致，本次无需写入。"}</Banner>}
      <div className="file-plans">{plan.files.map((file) => <article className={`file-plan${file.blocked ? " blocked" : ""}`} key={`${file.path}:${file.clientId}`}><div className="file-plan-head"><AgentBadge clientId={file.clientId} size="md" /><b>{file.clientName}</b><div className="spacer" /><Pill tone={file.blocked ? "warn" : file.willWrite ? "ok" : "plain"}>{file.blocked ? "已阻止" : file.willWrite ? file.existed ? "更新" : "新建" : "无变化"}</Pill></div>
        <code className="actual-file-path">{file.path}</code><div className="file-plan-meta"><span>{file.scope === "user" ? "全局配置" : file.scope === "project" ? "项目配置" : "本地配置"}</span><span>{file.format}</span><span>新增 {file.newCount} · 更新 {file.updateCount} · 保持 {file.unchangedCount}</span></div>
        {file.blocked && <Banner tone="warn">{file.blocked}</Banner>}
        {file.warnings.map((warning) => <Banner tone="warn" key={warning}>{warning}</Banner>)}
        {file.diff && <details className="raw-diff"><summary>查看配置差异</summary><DiffBody file={file} /><p className="field-hint">普通配置正常显示，识别到的密钥、Token、密码及认证信息已隐藏。</p></details>}
      </article>)}</div>
    </>}
    <footer className="editor-footer preview-footer"><span className="field-hint"><Icon name="shield" size={15} />保存原文备份</span><div className="spacer" /><button className="btn" disabled={applying} onClick={onBack}>返回修改</button>
      {plan && <button className="btn primary" disabled={applying || planning || !writable.length} onClick={onApply}>{applying ? "正在写入…" : `确认写入 ${writable.length} 个文件`}</button>}</footer>
  </section>;
}
