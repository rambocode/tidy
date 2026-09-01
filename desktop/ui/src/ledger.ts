// The house ledger: a small local record of what Tidy has actually done on
// this machine. The newspaper masthead and the hero statlines quote figures
// like "上次清理 · 14 天前" and "累计已省 · 312 GB"; those numbers must come
// from real runs, so every destructive flow and every maintenance task writes
// one entry here. Nothing is ever invented — an empty ledger renders "—".

/** One reclaim entry, written when an execution flow finishes. */
export interface ReclaimEntry {
  /** Which surface produced it: the clean tab, an uninstall, or analyze. */
  kind: "clean" | "apps" | "analyze";
  /** Epoch ms. */
  at: number;
  freedKb: number;
}

/** One maintenance entry, written per optimize task run. */
export interface MaintenanceEntry {
  /** Backend task id (e.g. "purgeable"). */
  id: string;
  /** Localised task title at the time of the run. */
  title: string;
  at: number;
  outcome: string;
}

const RECLAIM_KEY = "tidy.ledger.reclaim.v1";
const MAINT_KEY = "tidy.ledger.maintenance.v1";
/** Entries kept per ledger; older ones are dropped on write. */
const MAX_ENTRIES = 200;

/** Read a JSON array, tolerating absent/corrupt storage. */
function read<T>(key: string): T[] {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as T[]) : [];
  } catch {
    return [];
  }
}

/** Write back the newest MAX_ENTRIES rows (quota failures are non-fatal). */
function write<T>(key: string, rows: T[]): void {
  try {
    localStorage.setItem(key, JSON.stringify(rows.slice(-MAX_ENTRIES)));
  } catch {
    /* private mode or quota: the ledger is a convenience, never a source of truth */
  }
}

/** Record freed space after an execution flow completed. */
export function recordReclaim(kind: ReclaimEntry["kind"], freedKb: number): void {
  if (freedKb <= 0) return;
  write(RECLAIM_KEY, [...read<ReclaimEntry>(RECLAIM_KEY), { kind, at: Date.now(), freedKb }]);
}

/** Record one maintenance task result. */
export function recordMaintenance(id: string, title: string, outcome: string): void {
  write(MAINT_KEY, [...read<MaintenanceEntry>(MAINT_KEY), { id, title, at: Date.now(), outcome }]);
}

export const reclaimLog = (): ReclaimEntry[] => read<ReclaimEntry>(RECLAIM_KEY);
export const maintenanceLog = (): MaintenanceEntry[] => read<MaintenanceEntry>(MAINT_KEY);

/** Newest reclaim of one kind, or null when that surface has never run. */
export function lastReclaim(kind: ReclaimEntry["kind"]): ReclaimEntry | null {
  const rows = reclaimLog().filter((r) => r.kind === kind);
  return rows.length > 0 ? rows[rows.length - 1] : null;
}

/** Total space reclaimed across every surface, in KB. */
export function totalFreedKb(): number {
  return reclaimLog().reduce((sum, r) => sum + r.freedKb, 0);
}

/** Newest maintenance entry for a task id, or null when never run. */
export function lastMaintenance(id?: string): MaintenanceEntry | null {
  const rows = id ? maintenanceLog().filter((m) => m.id === id) : maintenanceLog();
  return rows.length > 0 ? rows[rows.length - 1] : null;
}

/** How many maintenance tasks have completed successfully, ever. */
export function maintenanceDoneCount(): number {
  return maintenanceLog().filter((m) => m.outcome === "ok" || m.outcome === "unchanged").length;
}

/** Whole days between an epoch-ms stamp and now (0 = today). */
export function daysSince(at: number): number {
  return Math.floor((Date.now() - at) / 86_400_000);
}
