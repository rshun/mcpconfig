import { invoke } from "@tauri-apps/api/core";
import type {
  AppInfoDto,
  BackupDto,
  CatalogDto,
  ClientDto,
  ExistingConfigDto,
  MatrixDto,
  PlanInput,
  PlanDto,
  ReceiptDto,
  Scope,
  GuiSettings,
  ImportChoice, ImportPreview, ImportResult, ImportScan,
} from "./types";
import { mockCall, mockMode } from "./mock";

/** 是否运行在 Tauri 容器内 */
export const isDesktop =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** 浏览器预览时走 mock，便于评审 UI */
export const isMock = !isDesktop || mockMode;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isDesktop && !mockMode) {
    return invoke<T>(cmd, args);
  }
  return mockCall<T>(cmd, args);
}

export const api = {
  appInfo: () => call<AppInfoDto>("app_info"),
  loadSettings: () => call<GuiSettings>("load_settings"),
  defaultSettings: () => call<GuiSettings>("default_settings"),
  saveSettings: (settings: GuiSettings) => call<string[]>("save_settings", { settings }),
  chooseDirectory: (initial: string | null) => call<string | null>("choose_directory", { initial }),
  createCatalog: (path: string) => call<string[]>("create_catalog", { path }),
  scanImport: (catalogPath: string, clientId: string, scope: Scope, projectRoot: string | null) =>
    call<ImportScan>("scan_import", { catalogPath, clientId, scope, projectRoot }),
  previewImport: (scanId: string, choices: ImportChoice[], scope: Scope, projectRoot: string | null) =>
    call<ImportPreview>("preview_import", { scanId, choices, scope, projectRoot }),
  commitImport: (previewId: string, scope: Scope, projectRoot: string | null) =>
    call<ImportResult>("commit_import", { previewId, scope, projectRoot }),
  cancelImport: (scanId: string) => call<void>("cancel_import", { scanId }),
  detect: (scope: Scope, projectRoot: string | null = null) => call<ClientDto[]>("detect", { scope, projectRoot }),
  readClientConfig: (clientId: string, scope: Scope, projectRoot: string | null = null) =>
    call<ExistingConfigDto>("read_client_config", { clientId, scope, projectRoot }),
  matrix: (catalogPath: string, scope: Scope, projectRoot: string | null = null) =>
    call<MatrixDto>("matrix", { catalogPath, scope, projectRoot }),
  loadCatalog: (path: string) => call<CatalogDto>("load_catalog", { path }),
  saveCatalog: (path: string, catalog: unknown) =>
    call<string[]>("save_catalog", { path, catalog }),
  plan: (request: PlanInput) => call<PlanDto>("plan", { ...request }),
  apply: (request: PlanInput, dryRun: boolean) =>
    call<ReceiptDto>("apply", {
      ...request,
      dryRun,
    }),
  backups: () => call<BackupDto[]>("backups"),
  restore: (id: string) => call<number>("restore", { id }),
};

/** 默认清单路径（首次启动用示例） */
export const DEFAULT_CATALOG = "examples/customer-mcp.yaml";
