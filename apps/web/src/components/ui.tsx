import type { ReactNode } from "react";

export function Panel({
  title,
  actions,
  children,
  tight,
}: {
  title?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  tight?: boolean;
}) {
  return (
    <div className="panel">
      {(title || actions) && (
        <div className="panel-head">
          {title}
          <div className="spacer" />
          {actions}
        </div>
      )}
      <div className={tight ? "panel-body tight" : "panel-body"}>{children}</div>
    </div>
  );
}

export function Pill({
  tone = "plain",
  children,
}: {
  tone?: "plain" | "ok" | "warn" | "danger" | "info";
  children: ReactNode;
}) {
  return <span className={`pill ${tone}`}>{children}</span>;
}

export function Banner({
  tone = "info",
  children,
}: {
  tone?: "info" | "warn" | "ok" | "danger";
  children: ReactNode;
}) {
  return <div className={`banner ${tone}`}>{children}</div>;
}

export function Empty({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="center-box">
      <div className="big">{title}</div>
      {hint && <div>{hint}</div>}
    </div>
  );
}

export function Loading({ rows = 3 }: { rows?: number }) {
  return (
    <div style={{ display: "grid", gap: 8, padding: 16 }}>
      {Array.from({ length: rows }).map((_, i) => (
        <div key={i} className="skeleton" style={{ width: `${90 - i * 12}%` }} />
      ))}
    </div>
  );
}

export function Field({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div className="form-row">
      <label className="label">{label}</label>
      {children}
    </div>
  );
}
