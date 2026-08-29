// Typed invoke wrappers: one function per Tauri command, so views never call
// invoke() with a raw string. Progress streams use Tauri channels.

import { Channel, invoke } from "@tauri-apps/api/core";
import type { CelestialCatalog } from "./explore";
import type {
  AppInfo,
  AppMeta,
  CachedUpdates,
  CleanPlanPayload,
  DirListing,
  EmbeddedLoginItem,
  ExecItem,
  ExecReport,
  OptimizeTask,
  PlanSummary,
  ProcessDetail,
  PurgePlanPayload,
  ScanEvent,
  StatusSnapshot,
  TaskResult,
  LoginItem,
  TouchIdStatus,
  UninstallPlanPayload,
  UpdateCatalog,
  UpdateResult,
  WhitelistPayload,
} from "./types";

/** Build a progress channel from a plain callback. */
function channel<T>(cb: (event: T) => void): Channel<T> {
  const ch = new Channel<T>();
  ch.onmessage = cb;
  return ch;
}

/** Session-unique task id for cancellable scans. */
let taskCounter = 0;
export const newTaskId = () => `task-${++taskCounter}-${Date.now()}`;

export const appMeta = () => invoke<AppMeta>("app_meta");
export const statusSnapshot = () => invoke<StatusSnapshot>("status_snapshot");

export const whitelistGet = () => invoke<WhitelistPayload>("whitelist_get");
export const whitelistSet = (patterns: string[]) =>
  invoke<string[]>("whitelist_set", { patterns });
export const purgePathsGet = () => invoke<string[] | null>("purge_paths_get");
export const touchidStatus = () => invoke<TouchIdStatus>("touchid_status");
export const celestialCatalog = () => invoke<CelestialCatalog>("celestial_catalog");

export const analyzeScan = (
  root: string,
  taskId: string,
  onProgress: (e: ScanEvent) => void,
) =>
  invoke<DirListing>("analyze_scan", {
    root,
    taskId,
    onProgress: channel(onProgress),
  });

export const planDeletePaths = (paths: string[], taskId: string) =>
  invoke<PlanSummary>("plan_delete_paths", { paths, taskId });

export const planClean = (taskId: string, onProgress: (e: ScanEvent) => void) =>
  invoke<CleanPlanPayload>("plan_clean", {
    taskId,
    onProgress: channel(onProgress),
  });

export const revealInFinder = (path: string) =>
  invoke<void>("reveal_in_finder", { path });

export const listApps = (taskId: string) =>
  invoke<AppInfo[]>("list_apps", { taskId });

export const planUninstall = (appPaths: string[], taskId: string) =>
  invoke<UninstallPlanPayload>("plan_uninstall", { appPaths, taskId });

export const planPurge = (taskId: string, onProgress: (e: ScanEvent) => void) =>
  invoke<PurgePlanPayload>("plan_purge", { taskId, onProgress: channel(onProgress) });

export const planInstaller = (taskId: string) =>
  invoke<PlanSummary>("plan_installer", { taskId });

export const executePlan = (
  planId: string,
  selection: string[],
  dryRun: boolean,
  onProgress: (item: ExecItem) => void,
  trashOverride: boolean | null = null,
) =>
  invoke<ExecReport>("execute_plan", {
    planId,
    selection,
    dryRun,
    trashOverride,
    onProgress: channel(onProgress),
  });

export const cancelTask = (taskId: string) =>
  invoke<boolean>("cancel_task", { taskId });

export const listOptimizeTasks = () => invoke<OptimizeTask[]>("list_optimize_tasks");
export const runOptimize = (taskIds: string[]) =>
  invoke<TaskResult[]>("run_optimize", { taskIds });

export const appIcon = (appPath: string) =>
  invoke<string | null>("app_icon", { appPath });

export const previewUninstall = (appPaths: string[], taskId: string) =>
  invoke<UninstallPlanPayload>("preview_uninstall", { appPaths, taskId });

/** Newest known catalog without scanning (memory or persisted last scan). */
export const cachedAppUpdates = () => invoke<CachedUpdates | null>("cached_app_updates");

export const listAppUpdates = (taskId: string, force = false) =>
  invoke<UpdateCatalog>("list_app_updates", { taskId, force });

export const runAppUpdates = (updateIds: string[], taskId: string) =>
  invoke<UpdateResult[]>("run_app_updates", { updateIds, taskId });

export const setAppUpdateIgnored = (updateId: string, ignored: boolean) =>
  invoke<void>("set_app_update_ignored", { updateId, ignored });

export const listLoginItems = () => invoke<LoginItem[]>("list_login_items");

export const listEmbeddedLoginItems = () =>
  invoke<EmbeddedLoginItem[]>("list_embedded_login_items");

export const setLoginItemEnabled = (
  label: string,
  path: string,
  scope: string,
  enable: boolean,
) => invoke<void>("set_login_item_enabled", { label, path, scope, enable });

export const processDetail = (pid: number) =>
  invoke<ProcessDetail | null>("process_detail", { pid });

export const signalProcess = (pid: number, force: boolean) =>
  invoke<void>("signal_process", { pid, force });

export const fdaStatus = () => invoke<boolean>("fda_status");
export const openFdaSettings = () => invoke<void>("open_fda_settings");
export const autostartGet = () => invoke<boolean>("autostart_get");
export const autostartSet = (enable: boolean) => invoke<void>("autostart_set", { enable });
export const traySetVisible = (visible: boolean) => invoke<void>("tray_set_visible", { visible });
export const setKeepInTray = (enable: boolean) => invoke<void>("set_keep_in_tray", { enable });
