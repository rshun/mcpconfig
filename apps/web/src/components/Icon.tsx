export type IconName = "library" | "agents" | "history" | "settings" | "plus" | "import" | "folder" | "refresh" | "close" | "arrow" | "search" | "chevron" | "command" | "globe" | "check" | "edit" | "grid" | "shield";
const paths: Record<IconName, string[]> = {
  library: ["M4 4h6v16H4zM14 4h6v16h-6zM4 8h6M14 8h6"],
  agents: ["M3 4h18v12H3zM8 20h8M12 16v4"], history: ["M3 11a9 9 0 1 1 2.6 7M3 4v7h7M12 7v5l3 2"],
  settings: ["M4 7h16M4 17h16M8 4v6M16 14v6"], plus: ["M12 5v14M5 12h14"],
  import: ["M12 3v12M7 10l5 5 5-5M4 16v5h16v-5"], folder: ["M3 6h7l2 2h9v12H3z"],
  refresh: ["M20 10a8 8 0 0 0-14-5L3 8M3 3v5h5M4 14a8 8 0 0 0 14 5l3-3M16 16h5v5"],
  close: ["M6 6l12 12M18 6 6 18"], arrow: ["M4 12h16M14 6l6 6-6 6"], search: ["M21 21l-5-5", "M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0"],
  chevron: ["M9 5l7 7-7 7"], command: ["m8 7-5 5 5 5M13 17h8"], globe: ["M3 12h18M12 3c6 6 6 12 0 18-6-6-6-12 0-18", "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0"],
  check: ["m5 12 4 4 10-10"], edit: ["m15 4 5 5M4 20l2-7L17 2l5 5-11 11z"], grid: ["M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z"],
  shield: ["M12 3 3 7v5c0 5 9 10 9 10s9-5 9-10V7zM8 12l3 3 5-6"],
};
export function Icon({ name, size = 18 }: { name: IconName; size?: number }) {
  return <svg className="icon" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">{paths[name].map((d, index) => <path d={d} key={index} />)}</svg>;
}
