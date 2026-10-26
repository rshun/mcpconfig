import { invoke } from "@tauri-apps/api/core";
import type {
  AppInfoDto,
  BackupDto,
  CatalogDto,
  ClientDto,
  MatrixDto,
  MergeMode,
  PlanDto,
  ReceiptDto,
  Scope,
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
  detect: (scope: Scope) => call<ClientDto[]>("detect", { scope }),
  matrix: (catalogPath: string, scope: Scope) =>
    call<MatrixDto>("matrix", { catalogPath, scope }),
  loadCatalog: (path: string) => call<CatalogDto>("load_catalog", { path }),
  saveCatalog: (path: string, catalog: unknown) =>
    call<string[]>("save_catalog", { path, catalog }),
  plan: (
    catalogPath: string,
    clients: string[] | null,
    scope: Scope,
    mode: MergeMode,
    allowCommentStrip: boolean
  ) =>
    call<PlanDto>("plan", {
      catalogPath,
      clients,
      scope,
      mode,
      allowCommentStrip,
    }),
  apply: (
    catalogPath: string,
    clients: string[] | null,
    scope: Scope,
    mode: MergeMode,
    allowCommentStrip: boolean,
    dryRun: boolean
  ) =>
    call<ReceiptDto>("apply", {
      catalogPath,
      clients,
      scope,
      mode,
      allowCommentStrip,
      dryRun,
    }),
  backups: () => call<BackupDto[]>("backups"),
  restore: (id: string) => call<number>("restore", { id }),
};

/** 默认清单路径（首次启动用示例） */
export const DEFAULT_CATALOG = "examples/customer-mcp.yaml";
