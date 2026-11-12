import { useEffect, useId, useRef, type ReactNode } from "react";
import { Icon } from "./Icon";

export function Dialog({ title, children, onClose, busy = false, wide = false }: {
  title: string; children: ReactNode; onClose: () => void; busy?: boolean; wide?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const id = useId();
  useEffect(() => {
    const dialog = ref.current!;
    if (!dialog.open) {
      dialog.showModal();
      const explicitTarget = dialog.querySelector<HTMLElement>("[autofocus]");
      if (explicitTarget) {
        explicitTarget.focus();
      } else {
        dialog.focus();
      }
    }
    return () => { if (dialog.open) dialog.close(); };
  }, []);
  return <dialog ref={ref} tabIndex={-1} className={`dialog${wide ? " dialog-wide" : ""}`} aria-labelledby={id}
    onKeyDown={(event) => {
      if (event.key !== "Escape") return;
      // 在浏览器执行原生关闭前先经过草稿/写入保护；cancel 仍处理其他原生关闭请求。
      event.preventDefault(); event.stopPropagation();
      if (!busy) onClose();
    }}
    onCancel={(event) => { event.preventDefault(); if (!busy) onClose(); }}
    onClick={(event) => {
      if (event.target !== ref.current || busy) return;
      const rect = ref.current.getBoundingClientRect();
      if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) onClose();
    }}>
    <header className="dialog-header"><div><span className="eyebrow">MCP HUB</span><h2 id={id}>{title}</h2></div>
      <button className="btn icon-button ghost" aria-label={`关闭${title}`} disabled={busy} onClick={onClose}><Icon name="close" /></button></header>
    <div className="dialog-content">{children}</div>
  </dialog>;
}
