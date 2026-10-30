import type { Entry } from "./pages/Catalog";
import type { Scope } from "./types";

export function scopeReason(entry: Entry, scope: Scope): string | null {
  if (!entry.scope || entry.scope === "inherit" || entry.scope === scope) return null;
  return entry.scope === "local" ? "此配置仅用于本地范围，当前 GUI 暂不支持" : `仅适用于${entry.scope === "user" ? "全局" : "项目"}配置，请切换工作范围`;
}
export function scopeLabel(entry: Entry): string {
  return !entry.scope || entry.scope === "inherit" ? "跟随工作范围" : entry.scope === "user" ? "仅全局" : entry.scope === "project" ? "仅项目" : "仅本地";
}
export function variableNames(entry: Entry): string[] {
  const names = new Set<string>();
  for (const match of JSON.stringify(entry).matchAll(/\$\{(?:ENV:)?([A-Za-z_][A-Za-z0-9_]*)\}/g)) names.add(match[1]);
  if (entry.auth?.token_env && /^[A-Za-z_][A-Za-z0-9_]*$/.test(entry.auth.token_env)) names.add(entry.auth.token_env);
  return [...names];
}
export function fileName(path: string): string {
  return path.replace(/\\/g, "/").split("/").filter(Boolean).at(-1) || path;
}
