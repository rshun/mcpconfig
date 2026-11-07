export type Scope = "user" | "project" | "local";
export type MergeMode = "merge" | "replace";

export interface GuiSettings {
  version: number;
  catalogPath: string;
  backupDir: string;
  defaultScope: "user" | "project";
  defaultMode: MergeMode;
  lastProjectDir: string | null;
}

/** 客户端 id → 该客户端本次选中的 MCP 名称；空映射不选择任何配置。 */
export type McpSelection = Record<string, string[]>;

export interface PlanInput {
  catalogPath: string;
  selection: McpSelection;
  scope: Scope;
  mode: MergeMode;
  allowCommentStrip: boolean;
  projectRoot?: string | null;
}

export type CellState = "ok" | "absent" | "diff" | "unsupported";
export type DetectState = "ready" | "installed" | "configOnly" | "missing";

export interface ClientDto {
  id: string;
  name: string;
  kind: string;
  format: string;
  state: DetectState;
  stateLabel: string;
  binary: string | null;
  path: string | null;
  configured: number;
  scopes: Scope[];
  scopeBlock: string | null;
}

export interface CellDto {
  clientId: string;
  state: CellState;
  detail: string | null;
}

export interface ExistingConfigDto {
  clientId: string;
  clientName: string;
  scope: Scope;
  path: string;
  format: string;
  existed: boolean;
  entries: { name: string; config: Record<string, unknown> }[];
}
export type ImportAction = "skip" | "add" | "rename" | "replace";
export interface ImportChoice {
  sourceName: string;
  action: ImportAction;
  targetName: string;
  /** 映射为环境变量的凭据字段 → 变量名 */
  bindings: Record<string, string>;
  /** 原样复制原值的凭据字段；每个凭据字段只能出现在 bindings 或 copyFields 之一 */
  copyFields: string[];
}
export interface ImportScan {
  scanId: string;
  sourcePath: string;
  catalogPath: string;
  catalogExists: boolean;
  items: { name: string; transport: string | null; conflict: boolean; blocked: string | null; fields: { id: string; label: string }[] }[];
}
export interface ImportPreview {
  previewId: string;
  catalogPath: string;
  changes: { sourceName: string; targetName: string; action: ImportAction; before: unknown | null; after: unknown }[];
}
export interface ImportResult { imported: number; warnings: string[] }
export interface RowDto {
  name: string;
  transport: string;
  scope: Scope;
  cells: CellDto[];
}
export interface ClientBriefDto {
  id: string;
  name: string;
  kind: string;
}
export interface MatrixDto {
  platform: string;
  scope: Scope;
  clients: ClientBriefDto[];
  rows: RowDto[];
  issues: string[];
}

export interface FileDto {
  clientId: string;
  clientName: string;
  path: string;
  format: string;
  scope: Scope;
  existed: boolean;
  blocked: string | null;
  newCount: number;
  updateCount: number;
  unchangedCount: number;
  diff: string;
  warnings: string[];
  willWrite: boolean;
}
export interface SkippedDto {
  clientId: string;
  reason: string;
}
export interface PlanDto {
  previewId: string;
  entryCount: number;
  issues: string[];
  missingVars: string[];
  skipped: SkippedDto[];
  files: FileDto[];
  writable: number;
}

export interface OutcomeDto {
  clientId: string;
  path: string;
  status: string;
  label: string;
  message: string | null;
}
export interface ReceiptDto {
  written: number;
  blocked: number;
  failed: number;
  backupId: string | null;
  outcomes: OutcomeDto[];
}

export interface BackupDto {
  id: string;
  createdAt: string;
  files: number;
  paths: string[];
}

export interface CatalogDto {
  exists?: boolean;
  catalog: any;
  issues: string[];
  entryCount: number;
}

export interface AppInfoDto {
  version: string;
  platform: string;
  home: string;
  projectRoot: string;
  dataDir: string;
}
