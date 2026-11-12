import type { Entry } from "./pages/Catalog";
import type { Scope } from "./types";

export function scopeReason(entry: Entry, scope: Scope): string | null {
  if (!entry.scope || entry.scope === "inherit" || entry.scope === scope) return null;
  const label = entry.scope === "user" ? "全局" : entry.scope === "project" ? "项目" : "本地";
  return `仅适用于${label}配置，请切换工作范围`;
}
export function scopeLabel(entry: Entry): string {
  return !entry.scope || entry.scope === "inherit" ? "跟随工作范围" : entry.scope === "user" ? "仅全局" : entry.scope === "project" ? "仅项目" : "仅本地";
}
export function variableNames(entry: Entry): string[] {
  const names = new Set<string>();
  const builtins = new Set(["HOME", "HOME_DIR", "PROJECT_ROOT", "CWD"]);
  for (const match of JSON.stringify(entry).matchAll(/\$\{(?:ENV:|env:)?([A-Za-z_][A-Za-z0-9_]*)\}/g)) {
    if (!builtins.has(match[1])) names.add(match[1]);
  }
  if (entry.auth?.type === "bearer" && entry.auth.token_env && /^[A-Za-z_][A-Za-z0-9_]*$/.test(entry.auth.token_env)) {
    names.add(entry.auth.token_env);
  }
  if (entry.auth && entry.auth.type !== "bearer") {
    const envRef = entry.auth.token_env || entry.auth.value_env;
    if (envRef && /^[A-Za-z_][A-Za-z0-9_]*$/.test(envRef)) names.add(envRef);
  }
  return [...names];
}
export function fileName(path: string): string {
  return path.replace(/\\/g, "/").split("/").filter(Boolean).at(-1) || path;
}
