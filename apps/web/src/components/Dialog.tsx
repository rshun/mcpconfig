import { useEffect, useId, useRef, type ReactNode } from "react";
import { Icon } from "./Icon";

export function Dialog({ title, children, onClose, busy = false, wide = false }: {
  title: string; children: ReactNode; onClose: () => void; busy?: boolean; wide?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const id = useId();
  useEffect(() => {
    const dialog = ref.current!;
    if (!dialog.open) dialog.showModal();
    return () => { if (dialog.open) dialog.close(); };
  }, []);
  return <dialog ref={ref} className={`dialog${wide ? " dialog-wide" : ""}`} aria-labelledby={id}
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
