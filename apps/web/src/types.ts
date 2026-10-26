export type Scope = "user" | "project" | "local";
export type MergeMode = "merge" | "replace";

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
