// Apps view (reference design): 卸载/更新/启动项 sub-tabs, real app icons,
// chevron-expandable leftover detail (expanding ≠ selecting), and a fixed
// footer bar with selected-app icons. Performance contract: the list renders
// ONCE per structural change; checkbox toggles patch only the affected row
// summary and the footer (no full re-render, no lag).

import {
  appIcon,
  cachedAppUpdates,
  cancelTask,
  listApps,
  listAppUpdates,
  listEmbeddedLoginItems,
  listLoginItems,
  newTaskId,
  planUninstall,
  previewUninstall,
  revealInFinder,
  runAppUpdates,
  setAppUpdateIgnored,
  setLoginItemEnabled,
} from "../ipc";
import { confirmSheet, runExecution } from "../flow";
import { esc, humanKb } from "../format";
import { t } from "../i18n";
import type { View } from "../router";
import type {
  AppInfo,
  AppUpdate,
  EmbeddedLoginItem,
  LoginItem,
  PlanItem,
  UninstallNote,
  UpdateCatalog,
  UpdateResult,
} from "../types";

/** Client-side icon cache: app path → data URL (null = known miss). */
const iconCache = new Map<string, string | null>();

/** Leftover detail cache: app path → items + notes. */
interface Detail {
  items: PlanItem[];
  notes: UninstallNote[];
}
const detailCache = new Map<string, Detail>();

/** Selection state: app path → checked item PATHS (stable across re-plans). */
/** 16px chevron for the row expand button (rotated via .chev.open). */
const CHEVRON_SVG =
  '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6l4 4 4-4"/></svg>';

const selected = new Map<string, Set<string>>();
/** Bundle size per app path (from list_apps) — the footer's provisional
 * total while an app's leftover scan is still running. */
const appSizes = new Map<string, number>();

type SubTab = "uninstall" | "updates" | "login";
let subTab: SubTab = "uninstall";
let sortKey: "name" | "size" = "size";
let updateTaskId: string | null = null;
let lastUpdateResults: UpdateResult[] = [];

export const uninstall: View = {
  mount(container) {
    selected.clear();
    void renderShell(container);
  },
};

/** Shell: sub-tabs + active tab body. */
async function renderShell(container: HTMLElement): Promise<void> {
  container.innerHTML = `
    <div class="content-narrow">
      <div class="flow-toolbar" id="subtabs">
        ${(["uninstall", "updates", "login"] as SubTab[])
          .map(
            (tab) =>
              `<button data-subtab="${tab}" class="${tab === subTab ? "danger" : ""}">${t(
                `apps.tab.${tab === "uninstall" ? "uninstall" : tab === "updates" ? "updates" : "login"}`,
              )}</button>`,
          )
          .join("")}
        <span style="flex:1"></span>
        <span id="tab-extra"></span>
      </div>
      <div id="tab-body"><div class="placeholder">…</div></div>
    </div>
    <div class="footer-bar" id="footer" style="display:none"></div>`;

  container.querySelectorAll<HTMLButtonElement>("button[data-subtab]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (updateTaskId) void cancelTask(updateTaskId);
      subTab = btn.dataset.subtab as SubTab;
      void renderShell(container);
    });
  });

  const body = container.querySelector<HTMLElement>("#tab-body")!;
  if (subTab === "uninstall") await renderUninstall(container, body);
  else if (subTab === "updates") await renderUpdates(body);
  else await renderLogin(container, body);
}

// ---------------------------------------------------------------------------
// 卸载
// ---------------------------------------------------------------------------

/** Uninstall tab: app rows with icons, chevron expand, footer selection bar. */
async function renderUninstall(container: HTMLElement, body: HTMLElement): Promise<void> {
  body.innerHTML = `<div class="placeholder">${t("apps.loading")}</div>`;
  const apps = await listApps(newTaskId());
  for (const app of apps) appSizes.set(app.path, app.size_kb);
  const extra = container.querySelector<HTMLElement>("#tab-extra")!;

  const drawList = () => {
    extra.innerHTML = `
      <span class="muted">${apps.length} ${t("apps.installed")}</span>
      <a href="#" data-sort="name" style="text-decoration:none" class="${sortKey === "name" ? "" : "muted"}">${t("apps.sort.name")} ⇅</a>
      <a href="#" data-sort="size" style="text-decoration:none" class="${sortKey === "size" ? "" : "muted"}">${t("apps.sort.size")} ⇅</a>`;
    extra.querySelectorAll<HTMLAnchorElement>("a[data-sort]").forEach((a) => {
      a.addEventListener("click", (ev) => {
        ev.preventDefault();
        sortKey = a.dataset.sort as "name" | "size";
        drawList();
      });
    });

    const sorted = [...apps].sort((a, b) =>
      sortKey === "size" ? b.size_kb - a.size_kb : a.name.localeCompare(b.name),
    );
    body.innerHTML = sorted.map((app) => rowHtml(app)).join("");
    sorted.forEach((app) => wireRow(container, body, app));
    void loadIcons(body, sorted);
    updateFooter(container, apps);
  };

  drawList();
}

/** Static HTML for one collapsed row. */
function rowHtml(app: AppInfo): string {
  const blocked = app.protected || app.official_uninstaller !== null;
  const why = app.protected
    ? t("apps.protected")
    : app.official_uninstaller
      ? `${t("apps.official")} (${app.official_uninstaller})`
      : "";
  return `<div class="app-row" data-row="${esc(app.path)}">
    <div class="head">
      <div class="avatar" data-avatar="${esc(app.path)}">${esc(app.name.slice(0, 1).toUpperCase())}</div>
      <div>
        <div class="title">${esc(app.name)}</div>
        <div class="meta">${esc(app.version || "—")} · ${humanKb(app.size_kb)}
          ${app.running ? ` · <span class="badge ok">${t("apps.active")}</span>` : ""}
          ${why ? ` · <span class="badge warn">${esc(why)}</span>` : ""}
        </div>
      </div>
      <div class="spacer"></div>
      <span class="summary" data-sum="${esc(app.path)}"></span>
      <button class="chev" data-chev="${esc(app.path)}" title="${t("apps.review.hint")}">${CHEVRON_SVG}</button>
      <input type="checkbox" data-app="${esc(app.path)}" ${blocked ? "disabled" : ""} />
    </div>
    <div class="detail" data-detail="${esc(app.path)}" style="display:none"></div>
  </div>`;
}

/** Wire one row's chevron + select checkbox (no re-render on toggle). */
function wireRow(container: HTMLElement, body: HTMLElement, app: AppInfo): void {
  const row = body.querySelector<HTMLElement>(`[data-row="${cssEscape(app.path)}"]`)!;
  const chev = row.querySelector<HTMLButtonElement>("button.chev")!;
  const detailEl = row.querySelector<HTMLElement>(".detail")!;
  const checkbox = row.querySelector<HTMLInputElement>("input[data-app]")!;

  chev.addEventListener("click", async () => {
    if (detailEl.style.display === "none") {
      chev.classList.add("open");
      detailEl.style.display = "";
      const detail = await loadDetail(app.path);
      renderDetail(container, detailEl, app, detail);
    } else {
      chev.classList.remove("open");
      detailEl.style.display = "none";
    }
  });

  /** Patch, never re-render: row summary + open detail + footer. */
  const refresh = () => {
    patchRowSummary(container, app.path);
    const detail = detailCache.get(app.path);
    if (detailEl.style.display !== "none" && detail) {
      renderDetail(container, detailEl, app, detail);
    }
    updateFooterFromDom(container);
  };

  checkbox.addEventListener("change", async () => {
    // Selected rows get the highlighted card look (see .app-row.selected).
    row.classList.toggle("selected", checkbox.checked);
    if (!checkbox.checked) {
      selected.delete(app.path);
      refresh();
      return;
    }
    // Optimistic: mark the app selected right away (the bundle itself is
    // always part of the plan) so the row and footer react instantly; the
    // leftover scan fills in the full item set when it returns.
    selected.set(app.path, new Set([app.path]));
    refresh();
    const detail = await loadDetail(app.path);
    // The user may have unchecked while the scan ran — do not resurrect.
    if (!checkbox.checked) return;
    selected.set(app.path, new Set(detail.items.map((i) => i.path)));
    refresh();
  });
}

/** Fetch (once) the leftover detail for an app. */
async function loadDetail(appPath: string): Promise<Detail> {
  const cached = detailCache.get(appPath);
  if (cached) return cached;
  const payload = await previewUninstall([appPath], newTaskId());
  const detail: Detail = {
    items: payload.summary.sections.flatMap((s) => s.items),
    notes: payload.notes,
  };
  detailCache.set(appPath, detail);
  return detail;
}

/** Leftover category derived from a path (display grouping only). */
function categoryOf(path: string): string {
  if (path.endsWith(".app")) return t("cat.app");
  if (path.includes("/Application Support/")) return t("cat.appsupport");
  if (path.includes("/Caches/")) return t("cat.cache");
  if (path.includes("/Logs/")) return t("cat.logs");
  if (path.includes("/Preferences/")) return t("cat.prefs");
  if (path.includes("/LaunchAgents/")) return t("cat.launchagents");
  if (path.includes("/HTTPStorages/")) return t("cat.httpstorage");
  if (path.includes("/WebKit/")) return t("cat.webkit");
  if (path.includes("/Saved Application State/")) return t("cat.savedstate");
  if (path.includes("/Containers/") || path.includes("/Group Containers/"))
    return t("cat.containers");
  return t("cat.other");
}

/** Render (or refresh) the expanded detail of one app: an inset card with
 * the app path, a "selected N/total" counter with a select-all toggle, the
 * auto-selected master checkbox, then one row per leftover item. */
function renderDetail(
  container: HTMLElement,
  el: HTMLElement,
  app: AppInfo,
  detail: Detail,
): void {
  const checkedSet = selected.get(app.path);
  const totalKb = detail.items.reduce((sum, i) => sum + (i.size_kb ?? 0), 0);
  const notes = detail.notes
    .map((n) => `<div class="badge warn">${esc(n.note)}</div>`)
    .join(" ");
  // Item checkboxes are only editable once the app itself is selected —
  // the row checkbox owns the selection set.
  const editable = !!checkedSet;

  const rows = detail.items
    .map(
      (i) => `<div class="det-item">
        <input type="checkbox" data-item="${esc(i.path)}" data-owner="${esc(app.path)}"
          ${checkedSet?.has(i.path) ? "checked" : ""} ${editable ? "" : "disabled"} />
        <span class="det-cat">${esc(categoryOf(i.path))}</span>
        <span class="det-path" title="${esc(i.path)}">${esc(i.path)}</span>
        <span class="det-size">${i.size_kb === null ? "?" : humanKb(i.size_kb)}</span>
      </div>`,
    )
    .join("");

  el.innerHTML = `
    ${notes ? `<div class="note-list">${notes}</div>` : ""}
    <div class="det-card">
      <div class="det-head">
        <div>
          <div class="det-name">${esc(app.name)}</div>
          <div class="det-apppath">${esc(app.path)}</div>
        </div>
        <div class="spacer"></div>
        <span class="det-count" data-count></span>
        <a href="#" class="det-toggle" data-toggle ${editable ? "" : "hidden"}></a>
      </div>
      <div class="det-auto">
        <strong>${t("apps.autoselect")}</strong>
        <span class="muted">${detail.items.length} ${t("apps.files")} · ${humanKb(totalKb)}</span>
        <div class="spacer"></div>
        <input type="checkbox" data-master ${editable ? "" : "disabled"} />
      </div>
      ${rows}
    </div>`;

  const items = [...el.querySelectorAll<HTMLInputElement>("input[data-item]")];
  const master = el.querySelector<HTMLInputElement>("input[data-master]")!;
  const count = el.querySelector<HTMLElement>("[data-count]")!;
  const toggle = el.querySelector<HTMLAnchorElement>("[data-toggle]")!;

  /** Sync the counter, the select-all link, and the tri-state master box. */
  const syncHead = () => {
    const set = selected.get(app.path);
    const n = set ? set.size : 0;
    const kb = detail.items
      .filter((i) => set?.has(i.path))
      .reduce((sum, i) => sum + (i.size_kb ?? 0), 0);
    count.textContent = `${t("apps.detail.count", { n, total: detail.items.length })} · ${humanKb(kb)}`;
    const all = n === detail.items.length;
    toggle.textContent = all ? t("apps.deselectAll") : t("apps.selectAll");
    master.checked = all;
    master.indeterminate = n > 0 && !all;
  };

  /** Push the selection set to the row summary and the footer. */
  const commit = () => {
    syncHead();
    patchRowSummary(container, app.path);
    updateFooterFromDom(container);
  };

  /** Set every item to one state (select-all link and master checkbox). */
  const setAll = (on: boolean) => {
    const set = selected.get(app.path);
    if (!set) return;
    for (const cb of items) {
      cb.checked = on;
      if (on) set.add(cb.dataset.item!);
      else set.delete(cb.dataset.item!);
    }
    commit();
  };

  items.forEach((cb) => {
    cb.addEventListener("change", () => {
      const set = selected.get(cb.dataset.owner!);
      if (!set) return;
      if (cb.checked) set.add(cb.dataset.item!);
      else set.delete(cb.dataset.item!);
      commit();
    });
  });
  master.addEventListener("change", () => setAll(master.checked));
  toggle.addEventListener("click", (ev) => {
    ev.preventDefault();
    setAll(!(selected.get(app.path)?.size === detail.items.length));
  });
  syncHead();
}

/** Patch one row's "N selected · size" summary span in place. */
function patchRowSummary(container: HTMLElement, appPath: string): void {
  const span = container.querySelector<HTMLElement>(`[data-sum="${cssEscape(appPath)}"]`);
  if (!span) return;
  const set = selected.get(appPath);
  const detail = detailCache.get(appPath);
  if (!set) {
    span.textContent = "";
    return;
  }
  if (!detail) {
    // Scan still running: show the bundle size as a provisional summary.
    span.textContent = `1 ${t("apps.selected")} · ${humanKb(appSizes.get(appPath) ?? 0)}`;
    return;
  }
  const kb = detail.items
    .filter((i) => set.has(i.path))
    .reduce((sum, i) => sum + (i.size_kb ?? 0), 0);
  span.textContent = `${set.size} ${t("apps.selected")} · ${humanKb(kb)}`;
}

/** Footer refresh helper reading current selection state. */
function updateFooterFromDom(container: HTMLElement): void {
  updateFooter(container, null);
}

/** Rebuild ONLY the footer bar (cheap; icons come from the client cache). */
function updateFooter(container: HTMLElement, _apps: AppInfo[] | null): void {
  const footer = container.querySelector<HTMLElement>("#footer");
  if (!footer) return;
  const paths = [...selected.keys()];
  if (paths.length === 0) {
    footer.style.display = "none";
    return;
  }
  let totalKb = 0;
  for (const [path, set] of selected) {
    const detail = detailCache.get(path);
    if (!detail) {
      // Leftover scan pending: count the bundle with its listed size.
      totalKb += appSizes.get(path) ?? 0;
      continue;
    }
    totalKb += detail.items
      .filter((i) => set.has(i.path))
      .reduce((sum, i) => sum + (i.size_kb ?? 0), 0);
  }
  footer.style.display = "";
  footer.innerHTML = `
    <span style="display:flex;gap:6px">${paths
      .map((p) => {
        const icon = iconCache.get(p);
        return icon
          ? `<img src="${icon}" style="width:26px;height:26px;border-radius:6px" alt="" />`
          : `<span class="avatar" style="width:26px;height:26px;font-size:12px;border-radius:6px">${esc(
              p.split("/").pop()?.slice(0, 1).toUpperCase() ?? "?",
            )}</span>`;
      })
      .join("")}</span>
    <span style="font-family:var(--font-mono);font-size:12px">${paths.length} App · ${humanKb(totalKb)}</span>
    <a href="#" id="clear-sel" style="font-size:12px">${t("apps.cancel")}</a>
    <button class="big-btn" id="remove">${esc(t("apps.removeN", { n: paths.length }))}</button>`;

  footer.querySelector("#clear-sel")!.addEventListener("click", (ev) => {
    ev.preventDefault();
    selected.clear();
    container
      .querySelectorAll<HTMLInputElement>("input[data-app]")
      .forEach((cb) => (cb.checked = false));
    container
      .querySelectorAll<HTMLElement>(".app-row.selected")
      .forEach((r) => r.classList.remove("selected"));
    container
      .querySelectorAll<HTMLElement>("[data-sum]")
      .forEach((s) => (s.textContent = ""));
    footer.style.display = "none";
  });

  footer.querySelector("#remove")!.addEventListener("click", async () => {
    // Apps whose leftover scan is still pending hold only the bundle path
    // (optimistic selection); resolve them so the full item set is removed.
    for (const path of paths) {
      if (detailCache.has(path)) continue;
      const detail = await loadDetail(path);
      if (selected.has(path)) selected.set(path, new Set(detail.items.map((i) => i.path)));
    }
    // Re-plan atomically across ALL selected apps at removal time; previews
    // are display state, never the executable plan.
    const payload = await planUninstall(paths, newTaskId());
    const wanted = new Set([...selected.values()].flatMap((set) => [...set]));
    const items = payload.summary.sections.flatMap((s) => s.items);
    const selection = items.filter((i) => wanted.has(i.path)).map((i) => i.id);
    const kb = items
      .filter((i) => wanted.has(i.path))
      .reduce((sum, i) => sum + (i.size_kb ?? 0), 0);
    const bodyEl = container.querySelector<HTMLElement>(".content-narrow")!;
    confirmSheet(selection.length, kb, t("apps.remove"), () => {
      footer.remove();
      runExecution(bodyEl, [{ planId: payload.summary.plan_id, selection }], {});
    });
  });
}

/** Load icons lazily with a small concurrency window; patch avatars in place. */
async function loadIcons(body: HTMLElement, apps: AppInfo[]): Promise<void> {
  const queue = [...apps];
  const worker = async () => {
    while (queue.length > 0) {
      const app = queue.shift()!;
      if (!iconCache.has(app.path)) {
        try {
          iconCache.set(app.path, await appIcon(app.path));
        } catch {
          iconCache.set(app.path, null);
        }
      }
      const icon = iconCache.get(app.path);
      if (icon) {
        const el = body.querySelector<HTMLElement>(
          `[data-avatar="${cssEscape(app.path)}"]`,
        );
        if (el) {
          el.style.background = "transparent";
          el.innerHTML = `<img src="${icon}" style="width:38px;height:38px;border-radius:9px" alt="" />`;
        }
      }
    }
  };
  await Promise.all([worker(), worker(), worker(), worker()]);
}

/** Escape a value for use inside an attribute selector. */
function cssEscape(value: string): string {
  return CSS.escape(value);
}

// ---------------------------------------------------------------------------
// 更新
// ---------------------------------------------------------------------------

/** Updates tab, reference design: "可在 Tidy 内更新 N 个" header with a 全部更新
 * link, flat icon rows showing installed → latest, and per-row 忽略更新 + 更新
 * actions. Loading contract: paint the newest known catalog instantly (stale
 * ids ⇒ actions disabled + a checking indicator), then run the real scan in
 * the background and swap it in. A live memory snapshot skips the rescan. */
async function renderUpdates(body: HTMLElement, force = false): Promise<void> {
  const taskId = newTaskId();
  updateTaskId = taskId;
  const extra = document.getElementById("tab-extra");
  if (extra) extra.innerHTML = "";
  // The app inventory only maps update names to app paths for icons; it
  // resolves independently so it never delays the first paint.
  const appsPromise = listApps(newTaskId()).catch(() => [] as AppInfo[]);
  const wireIcons = () =>
    void appsPromise.then((apps) => {
      const map = new Map(apps.map((app) => [app.name.toLowerCase(), app.path]));
      void loadUpdateIcons(body, map);
    });
  const showChecked = (catalog: UpdateCatalog) => {
    if (!extra) return;
    extra.innerHTML = `<span class="muted">${t("upd.checked")} ${new Date(
      catalog.checked_at * 1000,
    ).toLocaleTimeString()}</span> <button id="refresh-updates" title="${t("upd.refresh")}">⟳</button>`;
    extra.querySelector("#refresh-updates")?.addEventListener("click", () => {
      lastUpdateResults = [];
      void renderUpdates(body, true);
    });
  };

  // Instant first paint from the backend's cached catalog. A live snapshot is
  // current and actionable — render it and stop, no background rescan.
  let stalePainted = false;
  if (!force) {
    const cached = await cachedAppUpdates().catch(() => null);
    if (updateTaskId !== taskId) return;
    if (cached?.live) {
      updateTaskId = null;
      paintUpdates(body, cached.catalog, false);
      wireIcons();
      showChecked(cached.catalog);
      return;
    }
    if (cached && cached.catalog.updates.length > 0) {
      paintUpdates(body, cached.catalog, true);
      wireIcons();
      stalePainted = true;
      if (extra) {
        extra.innerHTML = `<span class="upd-checking"><span class="spin">⟳</span> ${t("upd.checking")}</span>`;
      }
    }
  }
  if (!stalePainted) {
    body.innerHTML = `<div class="placeholder">${t("upd.loading")} <button id="cancel-update-scan">${t("upd.cancel")}</button></div>`;
    body.querySelector("#cancel-update-scan")?.addEventListener("click", () => {
      void cancelTask(taskId);
    });
  }

  try {
    const catalog = await listAppUpdates(taskId, force);
    if (updateTaskId !== taskId) return;
    updateTaskId = null;
    paintUpdates(body, catalog, false);
    wireIcons();
    showChecked(catalog);
  } catch (e) {
    if (updateTaskId === taskId) updateTaskId = null;
    if (stalePainted) {
      // Background check failed or was cancelled: keep the cached list on
      // screen and just drop the checking indicator.
      if (extra) extra.innerHTML = "";
    } else {
      body.innerHTML = `<div class="placeholder">${esc(String((e as Error)?.message ?? e))}</div>`;
    }
  }
}

/** Build and wire the Updates body for one catalog. stale=true renders the
 * same layout with update/ignore actions disabled: those ids are not backed
 * by the backend's live authorization snapshot yet. */
function paintUpdates(body: HTMLElement, catalog: UpdateCatalog, stale: boolean): void {
  const visible = catalog.updates.filter((update) => !update.ignored);
  const hidden = catalog.updates.filter((update) => update.ignored);
  // Screenshot grouping: rows Tidy can act on vs. terminal-owned packages.
  const inMole = visible.filter((update) => update.action !== "terminal");
  const terminal = visible.filter((update) => update.action === "terminal");

  const rowHtml = (update: AppUpdate, hiddenRow: boolean): string => {
    const chev = update.release_notes
      ? `<button class="chev" data-notes="${esc(update.id)}" title="${t("upd.notes")}">›</button>`
      : "";
    const action = hiddenRow
      ? ""
      : update.action === "terminal"
        ? `<code class="upd-cmd" title="${t("upd.terminal")}">${esc(update.command_hint ?? t("upd.terminal"))}</code>`
        : `<button class="upd-btn" data-update="${esc(update.id)}" ${stale ? "disabled" : ""} title="${esc(actionLabel(update))}">${t("upd.update")}</button>`;
    return `<div class="upd-row">
      <div class="avatar" data-upname="${esc(update.name)}">${esc(update.name.slice(0, 1).toUpperCase())}</div>
      <div class="info">
        <div class="title-line">
          <span class="title">${esc(update.name)}</span>
          <span class="badge">${esc(sourceLabel(update.source))}</span>
          ${chev}
        </div>
        <div class="ver">
          <span class="old">${esc(update.installed)}</span>
          <span class="arrow">→</span>
          <span class="new">${esc(update.latest)}</span>
        </div>
        ${update.release_notes ? `<div class="notes" data-notesbody="${esc(update.id)}" style="display:none">${esc(update.release_notes)}</div>` : ""}
      </div>
      <span class="spacer"></span>
      <a href="#" class="upd-ignore ${stale ? "disabled" : ""}" data-ignore="${esc(update.id)}" data-ignored="${hiddenRow}">${
        hiddenRow ? t("upd.unignore") : t("upd.ignoreRow")
      }</a>
      ${action}
    </div>`;
  };

  const resultBlock = lastUpdateResults.length
    ? `<div class="panel">${lastUpdateResults
        .map(
          (result) =>
            `<div class="row"><strong>${esc(result.outcome)}</strong><span>${esc(result.message)}</span></div>`,
        )
        .join("")}</div>`
    : "";
  const warningBlock = catalog.warnings.length
    ? `<details class="panel"><summary>${t("upd.warnings")} (${catalog.warnings.length})</summary>
        ${catalog.warnings.map((warning) => `<div class="muted">${esc(warning)}</div>`).join("")}
      </details>`
    : "";
  const upToDateBlock = catalog.up_to_date.length
    ? `<details class="panel"><summary>${t("upd.uptodate")} (${catalog.up_to_date.length})</summary>
        ${catalog.up_to_date
          .map(
            (app) =>
              `<div class="row"><span>${esc(app.name)}</span><span class="muted">${esc(app.version)} · ${esc(sourceLabel(app.source))}</span></div>`,
          )
          .join("")}
      </details>`
    : "";
  const header = `
    <div class="upd-head">
      <span class="lead">${t("upd.available")}</span>
      <span class="count">${t("upd.count", { n: inMole.length })}</span>
      <span class="grow"></span>
      ${inMole.length && !stale ? `<a href="#" id="update-all" class="upd-all">${t("upd.updateAll")}</a>` : ""}
    </div>`;

  if (visible.length === 0 && hidden.length === 0) {
    body.innerHTML = `${resultBlock}${warningBlock}${upToDateBlock}<div class="placeholder">${
      catalog.warnings.length ? t("upd.noneKnown") : t("upd.none")
    }</div>`;
  } else {
    body.innerHTML = `${resultBlock}${warningBlock}
      ${header}
      ${inMole.map((update) => rowHtml(update, false)).join("")}
      ${terminal.length ? `<div class="upd-head sub"><span class="lead">${t("upd.packages")}</span><span class="count">${t("upd.count", { n: terminal.length })}</span></div>` : ""}
      ${terminal.map((update) => rowHtml(update, false)).join("")}
      ${terminal.length ? `<div class="muted" style="margin:4px 4px 10px">${t("upd.hint")}</div>` : ""}
      ${hidden.length ? `<div class="upd-head sub"><span class="lead">${t("upd.hidden")}</span><span class="count">${t("upd.count", { n: hidden.length })}</span></div>` : ""}
      ${hidden.map((update) => rowHtml(update, true)).join("")}
      ${upToDateBlock}`;
  }

  // Notes chevrons work in both modes; actions are wired only on live data.
  body.querySelectorAll<HTMLButtonElement>("button[data-notes]").forEach((button) => {
    button.addEventListener("click", () => {
      const notes = body.querySelector<HTMLElement>(
        `[data-notesbody="${cssEscape(button.dataset.notes!)}"]`,
      );
      if (!notes) return;
      const open = notes.style.display !== "none";
      notes.style.display = open ? "none" : "";
      button.classList.toggle("open", !open);
    });
  });
  if (stale) return;
  body.querySelector("#update-all")?.addEventListener("click", (ev) => {
    ev.preventDefault();
    if (window.confirm(t("upd.confirm", { n: inMole.length }))) {
      void runUpdateSelection(body, inMole.map((update) => update.id));
    }
  });
  body.querySelectorAll<HTMLButtonElement>("button[data-update]").forEach((button) => {
    button.addEventListener("click", () => {
      void runUpdateSelection(body, [button.dataset.update!]);
    });
  });
  body.querySelectorAll<HTMLAnchorElement>("a[data-ignore]").forEach((link) => {
    link.addEventListener("click", async (ev) => {
      ev.preventDefault();
      link.style.pointerEvents = "none";
      await setAppUpdateIgnored(link.dataset.ignore!, link.dataset.ignored !== "true");
      lastUpdateResults = [];
      await renderUpdates(body);
    });
  });
}

/** Fill update-row avatars via the name→path inventory map (lazy, cached). */
async function loadUpdateIcons(
  body: HTMLElement,
  appPathByName: Map<string, string>,
): Promise<void> {
  const targets = [...body.querySelectorAll<HTMLElement>("[data-upname]")];
  for (const el of targets) {
    const path = appPathByName.get((el.dataset.upname ?? "").toLowerCase());
    if (!path) continue;
    if (!iconCache.has(path)) {
      try {
        iconCache.set(path, await appIcon(path));
      } catch {
        iconCache.set(path, null);
      }
    }
    const icon = iconCache.get(path);
    if (icon && el.isConnected) {
      el.style.background = "transparent";
      el.innerHTML = `<img src="${icon}" alt="" />`;
    }
  }
}

async function runUpdateSelection(body: HTMLElement, ids: string[]): Promise<void> {
  if (ids.length === 0) return;
  body.querySelectorAll<HTMLButtonElement>("button").forEach((button) => {
    button.disabled = true;
  });
  const taskId = newTaskId();
  updateTaskId = taskId;
  body.innerHTML = `<div class="placeholder">${t("upd.running")} <button id="cancel-update-run">${t("upd.cancel")}</button></div>`;
  body.querySelector("#cancel-update-run")?.addEventListener("click", () => {
    void cancelTask(taskId);
  });
  try {
    lastUpdateResults = await runAppUpdates(ids, taskId);
  } catch (error) {
    lastUpdateResults = [
      {
        id: "request",
        outcome: "failed",
        cause: "request_failed",
        message: String((error as Error)?.message ?? error),
      },
    ];
  } finally {
    if (updateTaskId === taskId) updateTaskId = null;
  }
  await renderUpdates(body);
}

function sourceLabel(source: string): string {
  return t(`upd.source.${source}`);
}

function actionLabel(update: AppUpdate): string {
  if (update.action === "terminal") return t("upd.terminal");
  if (update.action === "open_app_store") return t("upd.openStore");
  if (update.action === "open_app") return t("upd.openApp");
  if (update.action === "open_website") return t("upd.openWebsite");
  return t("upd.update");
}

// ---------------------------------------------------------------------------
// 启动项
// ---------------------------------------------------------------------------

/** Login-items tab, reference design: app-embedded items (display-only, with
 * app icons) on top, then "Background Services N" launchd rows with working
 * enable/disable toggles (user scope; system scope needs the helper). */
async function renderLogin(container: HTMLElement, body: HTMLElement): Promise<void> {
  body.innerHTML = `<div class="placeholder">…</div>`;
  const [embedded, services] = await Promise.all([
    listEmbeddedLoginItems(),
    listLoginItems(),
  ]);
  if (embedded.length === 0 && services.length === 0) {
    body.innerHTML = `<div class="placeholder">${t("login.none")}</div>`;
    return;
  }

  const embeddedRows = embedded
    .map(
      (item: EmbeddedLoginItem, idx: number) => `<div class="li-row">
        <div class="li-icon" data-liicon="${idx}">⚙️</div>
        <div>
          <div class="li-title">${esc(item.app_name)} ${esc(item.item_name)}</div>
          <div class="li-sub">${item.kind === "login" ? t("login.kind.login") : t("login.kind.helper")} · ${t("login.embedded")}</div>
        </div>
        <div class="spacer" style="flex:1"></div>
        <button class="chev" data-reveal="${esc(item.app_path)}" title="Finder">📂</button>
        <button class="toggle" disabled></button>
      </div>`,
    )
    .join("");

  const serviceRows = services
    .map(
      (item: LoginItem, idx: number) => `<div class="li-row">
        <div class="li-icon">⚙️</div>
        <div>
          <div class="li-title">${esc(item.label)}</div>
          <div class="li-sub">${item.path.includes("LaunchDaemons") ? t("login.daemon") : t("login.agent")} · ${esc(item.program ?? item.label)}
            ${item.program !== null && !item.program_exists ? ` · <span class="badge danger">${t("login.orphan")}</span>` : ""}
          </div>
        </div>
        <div class="spacer" style="flex:1"></div>
        <button class="chev" data-reveal="${esc(item.path)}" title="Finder">📂</button>
        <button class="toggle ${item.enabled ? "on" : ""}" data-toggle="${idx}"
          ${item.scope === "system" ? `disabled title="${t("login.admin.toggle")}"` : ""}></button>
      </div>`,
    )
    .join("");

  body.innerHTML = `
    <div class="li-head">${t("login.items.head")}<span class="muted">${embedded.length} ${t("login.count")}</span></div>
    ${embeddedRows || `<div class="muted" style="padding:4px 16px">${t("login.none")}</div>`}
    <div class="li-head">${t("login.services.head")}<span class="muted">${services.length} ${t("login.count")}</span></div>
    ${serviceRows}
    <div class="muted" style="margin:10px 4px">${t("login.admin.toggle")}</div>`;

  // Reveal buttons.
  body.querySelectorAll<HTMLButtonElement>("button[data-reveal]").forEach((btn) => {
    btn.addEventListener("click", () => void revealInFinder(btn.dataset.reveal!));
  });

  // App icons for embedded rows (lazy, cached client-side).
  void (async () => {
    for (let i = 0; i < embedded.length; i++) {
      const path = embedded[i].app_path;
      if (!iconCache.has(path)) {
        try {
          iconCache.set(path, await appIcon(path));
        } catch {
          iconCache.set(path, null);
        }
      }
      const icon = iconCache.get(path);
      if (icon) {
        const el = body.querySelector<HTMLElement>(`[data-liicon="${i}"]`);
        if (el) el.innerHTML = `<img src="${icon}" alt="" />`;
      }
    }
  })();

  // Working toggles for user-scope services.
  body.querySelectorAll<HTMLButtonElement>("button[data-toggle]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const item = services[Number(btn.dataset.toggle)];
      const target = !btn.classList.contains("on");
      btn.disabled = true;
      try {
        await setLoginItemEnabled(item.label, item.path, item.scope, target);
        btn.classList.toggle("on", target);
      } catch {
        // Refusal (test guard / admin) leaves the visual state untouched.
      }
      btn.disabled = false;
    });
  });

  void container;
}
