export type ThemeMode = "light" | "dark";
export type AccentId = "ocean" | "violet" | "amber" | "emerald" | "rose" | "slate";
export type FontId = "wenkai" | "sans";
export type SurfaceStyleId = "3d" | "flat";

export interface AccentPreset {
  id: AccentId;
  label: string;
  desc: string;
  swatch: string;
}

export interface FontPreset {
  id: FontId;
  label: string;
  desc: string;
  sample: string;
}

export interface SurfaceStylePreset {
  id: SurfaceStyleId;
  label: string;
  desc: string;
}

export const ACCENT_PRESETS: AccentPreset[] = [
  { id: "ocean", label: "晴空蓝", desc: "清爽通透的现代蓝调", swatch: "#2563eb" },
  { id: "violet", label: "星云紫", desc: "柔和灵动的极光紫调", swatch: "#7c3aed" },
  { id: "amber", label: "暖砂橙", desc: "温润护眼的纸感暖调", swatch: "#d96b43" },
  { id: "emerald", label: "松石绿", desc: "沉稳清新的青翠绿调", swatch: "#0d9482" },
  { id: "rose", label: "樱桃红", desc: "明快鲜活的珊瑚粉调", swatch: "#e11d48" },
  { id: "slate", label: "极简灰", desc: "克制纯粹的黑白灰调", swatch: "#27272a" },
];

export const FONT_PRESETS: FontPreset[] = [
  { id: "wenkai", label: "温润文楷", desc: "现代西文搭配霞鹜文楷，温润护眼", sample: "Aa文" },
  { id: "sans", label: "现代黑体", desc: "现代无衬线字体栈，利落紧凑", sample: "Aa黑" },
];

export const SURFACE_STYLE_PRESETS: SurfaceStylePreset[] = [
  { id: "3d", label: "立体 3D", desc: "精工微立体斜面与真实按压手感" },
  { id: "flat", label: "简约扁平", desc: "轻盈克制的平面边框与投影" },
];

export function isAccentId(value: string | null): value is AccentId {
  return ACCENT_PRESETS.some((item) => item.id === value);
}

export function isFontId(value: string | null): value is FontId {
  return FONT_PRESETS.some((item) => item.id === value);
}

export function isSurfaceStyleId(value: string | null): value is SurfaceStyleId {
  return SURFACE_STYLE_PRESETS.some((item) => item.id === value);
}

export function readInitialTheme(): ThemeMode {
  const query = new URLSearchParams(window.location.search).get("theme");
  if (query === "dark" || query === "light") return query;
  try {
    const saved = localStorage.getItem("mcp-hub-theme");
    if (saved === "dark" || saved === "light") return saved;
  } catch {
    /* ignore */
  }
  return "light";
}

export function readInitialAccent(): AccentId {
  const query = new URLSearchParams(window.location.search).get("accent");
  if (isAccentId(query)) return query;
  try {
    const saved = localStorage.getItem("mcp-hub-accent");
    if (isAccentId(saved)) return saved;
  } catch {
    /* ignore */
  }
  return "ocean";
}

export function readInitialFont(): FontId {
  const query = new URLSearchParams(window.location.search).get("font");
  if (isFontId(query)) return query;
  try {
    const saved = localStorage.getItem("mcp-hub-font");
    if (isFontId(saved)) return saved;
  } catch {
    /* ignore */
  }
  return "wenkai";
}

export function readInitialSurfaceStyle(): SurfaceStyleId {
  const query = new URLSearchParams(window.location.search).get("style");
  if (isSurfaceStyleId(query)) return query;
  try {
    const saved = localStorage.getItem("mcp-hub-style");
    if (isSurfaceStyleId(saved)) return saved;
  } catch {
    /* ignore */
  }
  return "3d";
}
