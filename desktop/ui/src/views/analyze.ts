// Analyze view (reference design): gold particle sidebar with the directory
// list, breadcrumb on top, and a squarified treemap filling the main area. Clicking
// a block or sidebar row drills in; multi-select for Trash-only deletion.
// Completed listings are cached for the session and an in-flight scan keeps
// running across tab switches — re-entering the tab never restarts a scan.

import { analyzeScan, appMeta, cancelTask, newTaskId, planDeletePaths } from "../ipc";
import { renderFlow } from "../flow";
import { mountParticles } from "../particles";
import { esc, humanKb } from "../format";
import { t } from "../i18n";
import { squarify } from "../treemap";
import type { View } from "../router";
import type { DirListing } from "../types";

/** Warm block palette from the reference design, assigned by rank. */
const PALETTE = [
  "#c9a25e", "#c98b4a", "#b95f43", "#8f7a92", "#7c6f83",
  "#6e6258", "#a3865a", "#7f8a6a", "#9a6a55", "#6a7a8a",
];

/** Current directory (persists across navigations within the session). */
let currentRoot = "";
let home = "";

/** Cache of completed listings by root, persisted to localStorage so a
 * relaunch re-opens the last listings without re-scanning. Tab switches
 * re-render from here; dropped on manual refresh and cleared whenever a
 * trash flow starts, because deletion invalidates every cached size. */
const listingCache = new Map<string, DirListing>();
/** When each cached listing was scanned (epoch ms), shown as "cached at". */
const cachedAt = new Map<string, number>();

const STORE_KEY = "tidy.analyze.listings.v1";
/** Most roots to persist; oldest scans are evicted first. */
const STORE_MAX = 40;

/** Load the persisted cache once at module init (corrupt data is ignored). */
(function restoreCache() {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    if (!raw) return;
    const saved = JSON.parse(raw) as { root: string; at: number; listing: DirListing }[];
    for (const e of saved) {
      if (!e.root || !e.listing?.entries) continue;
      listingCache.set(e.root, e.listing);
      cachedAt.set(e.root, e.at);
    }
  } catch {
    /* ignore: cache is an optimisation only */
  }
})();

/** Write the cache back to localStorage (newest STORE_MAX roots). */
function persistCache(): void {
  try {
    const rows = [...listingCache.entries()]
      .map(([root, listing]) => ({ root, at: cachedAt.get(root) ?? 0, listing }))
      .sort((a, b) => b.at - a.at)
      .slice(0, STORE_MAX);
    localStorage.setItem(STORE_KEY, JSON.stringify(rows));
  } catch {
    /* quota or private mode: keep the in-memory cache only */
  }
}

/** Store a completed listing and persist. */
function cacheListing(root: string, listing: DirListing): void {
  listingCache.set(root, listing);
  cachedAt.set(root, Date.now());
  persistCache();
}

/** Drop one root (manual rescan) and persist. */
function dropListing(root: string): void {
  listingCache.delete(root);
  cachedAt.delete(root);
  persistCache();
}

/** Drop everything (after a deletion) and persist. */
function clearListings(): void {
  listingCache.clear();
  cachedAt.clear();
  persistCache();
}

/** "HH:MM" or "M/D HH:MM" for the cached-at label. */
function fmtTime(ms: number): string {
  const d = new Date(ms);
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  const today = new Date().toDateString() === d.toDateString();
  return today ? hm : `${d.getMonth() + 1}/${d.getDate()} ${hm}`;
}

/** The one in-flight scan, if any. It survives tab switches: the backend
 * keeps walking, progress accumulates in `label`, and the result lands in
 * the cache when done. Navigating to a different dir cancels it. */
let pending: { root: string; taskId: string; label: string } | null = null;

/** The container while this view is mounted; null lets a background scan
 * finish without touching another tab's DOM. */
let mounted: HTMLElement | null = null;

export const analyze: View = {
  async mount(container) {
    mounted = container;
    if (!home) home = (await appMeta()).home;
    if (mounted !== container) return; // switched away during the await
    if (!currentRoot) currentRoot = home;
    // Resume order: live scan → cached listing → fresh scan.
    if (pending) renderScanning(container, pending);
    else if (listingCache.has(currentRoot))
      renderListing(container, listingCache.get(currentRoot)!);
    else startScan(currentRoot);
  },
  unmount() {
    mounted = null;
  },
};

/** Navigate to a directory: serve from cache or start a scan. */
function openDir(root: string): void {
  currentRoot = root;
  const cached = listingCache.get(root);
  if (cached) {
    // Drop any in-flight scan too, or its completion would replace this view.
    if (pending) {
      void cancelTask(pending.taskId);
      pending = null;
    }
    if (mounted) renderListing(mounted, cached);
    return;
  }
  startScan(root);
}

/** Kick off a background scan of `root`, superseding any in-flight scan. */
function startScan(root: string): void {
  currentRoot = root;
  // The user navigated away from the old target — stop paying for it.
  if (pending) void cancelTask(pending.taskId);
  const state = { root, taskId: newTaskId(), label: "" };
  pending = state;
  if (mounted) renderScanning(mounted, state);
  analyzeScan(root, state.taskId, (e) => {
    state.label = e.label;
    if (pending !== state) return;
    const el = mounted?.querySelector<HTMLElement>("#scan-progress");
    if (el) el.textContent = e.label;
  })
    .then((listing) => {
      if (pending !== state) return; // superseded by a newer navigation
      pending = null;
      // Cancelled scans are incomplete — render once, never cache.
      if (!listing.truncated) cacheListing(root, listing);
      if (mounted) renderListing(mounted, listing);
    })
    .catch((e: unknown) => {
      if (pending !== state) return;
      pending = null;
      if (mounted)
        mounted.innerHTML = `<div class="placeholder">${esc(String((e as Error)?.message ?? e))}</div>`;
    });
}

/** Scanning hero with live progress; shown fresh or when returning mid-scan. */
function renderScanning(container: HTMLElement, state: { root: string; taskId: string; label: string }): void {
  container.innerHTML = `
    <div class="hero">
      <div class="sub">${t("ana.scanning")} ${esc(state.root)} … <span id="scan-progress">${esc(state.label)}</span></div>
      <button class="cta" id="cancel">${t("flow.cancel")}</button>
    </div>`;
  mountParticles(container.querySelector<HTMLElement>(".hero")!, "scan", "gold");
  container
    .querySelector("#cancel")!
    .addEventListener("click", () => void cancelTask(state.taskId));
}

/** Sidebar + treemap render. */
function renderListing(container: HTMLElement, listing: DirListing): void {
  const entries = listing.entries;
  const side = entries
    .map(
      (e, idx) => `<div class="item" data-dir="${e.is_dir ? esc(e.path) : ""}">
        <span style="color:${PALETTE[idx % PALETTE.length]}">📁</span>
        <div class="grow">
          <div class="name">${esc(e.name)}</div>
          <div class="size">${humanKb(e.size_kb)}</div>
        </div>
        <input type="checkbox" data-path="${esc(e.path)}" />
        ${e.is_dir ? `<span class="muted">›</span>` : ""}
      </div>`,
    )
    .join("");

  container.innerHTML = `
    <div class="ana-layout">
      <div class="ana-side">
        <div class="head">
          <div class="particle-box"></div>
          <div class="total">${entries.length} ${t("ana.items")} · ${humanKb(listing.total_kb)}</div>
        </div>
        <div class="muted" style="padding:4px 10px; font-size:11px">${t("ana.dir")}</div>
        ${side}
      </div>
      <div class="ana-main">
        <div class="crumbs">
          ${breadcrumbs(listing.root)}
          <span style="flex:1"></span>
          <span>${t("ana.current")} ${humanKb(listing.total_kb)}</span>
          ${listing.truncated ? `<span class="badge warn">${t("ana.cancelled")}</span>` : ""}
          ${cachedAt.has(listing.root) ? `<span class="muted">${t("ana.cached", { time: fmtTime(cachedAt.get(listing.root)!) })}</span>` : ""}
          <button id="rescan" title="${t("ana.refresh")}" style="padding:4px 10px">↻</button>
          <button id="trash-selected" disabled style="padding:4px 12px">${t("ana.trash")}</button>
        </div>
        <div class="treemap" id="treemap"></div>
        <div class="muted" style="margin-top:8px;font-size:11px">${t("ana.hint")}</div>
      </div>
    </div>`;

  // Small idle particle field where the Jupiter sphere used to sit.
  mountParticles(container.querySelector<HTMLElement>(".particle-box")!, "idle", "gold");

  renderTreemap(container, listing);

  // Navigation: sidebar rows and breadcrumb links.
  container.querySelectorAll<HTMLElement>(".ana-side .item").forEach((row) => {
    row.addEventListener("click", (ev) => {
      if ((ev.target as HTMLElement).tagName === "INPUT") return;
      const dir = row.dataset.dir;
      if (dir) openDir(dir);
    });
  });
  container.querySelectorAll<HTMLAnchorElement>("a.crumb").forEach((a) => {
    a.addEventListener("click", (ev) => {
      ev.preventDefault();
      openDir(a.dataset.dir!);
    });
  });
  container.querySelector("#rescan")!.addEventListener("click", () => {
    dropListing(listing.root);
    startScan(listing.root);
  });

  // Selection → Trash flow.
  const trashBtn = container.querySelector<HTMLButtonElement>("#trash-selected")!;
  const update = () => {
    trashBtn.disabled =
      container.querySelectorAll("input[data-path]:checked").length === 0;
  };
  container
    .querySelectorAll<HTMLInputElement>("input[data-path]")
    .forEach((cb) => cb.addEventListener("change", update));
  trashBtn.addEventListener("click", async () => {
    const paths = Array.from(
      container.querySelectorAll<HTMLInputElement>("input[data-path]:checked"),
    ).map((c) => c.dataset.path!);
    const meta = await appMeta();
    // Deletion invalidates every cached size along the ancestor chain, so the
    // whole cache goes, not just this root.
    clearListings();
    renderFlow(container, newTaskId(), {
      title: t("ana.trash"),
      verb: t("ana.trash"),
      helperAvailable: meta.helper_available,
      particles: "gold",
      plan: (taskId) => planDeletePaths(paths, taskId),
    });
  });
}

/** Fill the treemap area: top blocks + one aggregate block for the tail. */
function renderTreemap(container: HTMLElement, listing: DirListing): void {
  const box = container.querySelector<HTMLElement>("#treemap")!;
  const W = box.clientWidth || 700;
  const H = box.clientHeight || 460;

  const TOP = 9;
  const shown = listing.entries.slice(0, TOP).filter((e) => e.size_kb > 0);
  const rest = listing.entries.slice(TOP);
  const restKb = rest.reduce((sum, e) => sum + e.size_kb, 0);
  const weights = shown.map((e) => e.size_kb);
  if (restKb > 0) weights.push(restKb);
  const rects = squarify(weights, W, H);

  box.innerHTML = rects
    .map((r, idx) => {
      const isRest = idx >= shown.length;
      const entry = shown[idx];
      const label = isRest
        ? `${rest.length} ${t("ana.rest")}`
        : esc(entry.name);
      const size = isRest ? restKb : entry.size_kb;
      const showText = r.w > 70 && r.h > 40;
      return `<div class="block" data-dir="${!isRest && entry.is_dir ? esc(entry.path) : ""}"
        style="left:${r.x}px;top:${r.y}px;width:${r.w}px;height:${r.h}px;
        background:${isRest ? "#5c554d" : PALETTE[idx % PALETTE.length]}">
        ${showText ? `<div class="bname">${isRest ? "▦ " : "📁 "}${label}</div><div class="bsize">${humanKb(size)}</div>` : ""}
      </div>`;
    })
    .join("");

  box.querySelectorAll<HTMLElement>(".block").forEach((block) => {
    block.addEventListener("click", () => {
      const dir = block.dataset.dir;
      if (dir) openDir(dir);
    });
  });
}

/** Clickable breadcrumb chain; the home prefix collapses to the disk root. */
function breadcrumbs(root: string): string {
  const parts = root.split("/").filter(Boolean);
  let acc = "";
  const links = parts.map((part) => {
    acc += "/" + part;
    return `<a href="#" class="crumb" data-dir="${esc(acc)}">${esc(part)}</a> ›`;
  });
  return [`<a href="#" class="crumb" data-dir="/">⏏ ${t("ana.whole")}</a> ›`, ...links]
    .join(" ")
    .replace(/›\s*$/, "");
}
