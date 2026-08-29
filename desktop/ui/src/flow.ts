// The shared destructive-flow skeleton: scanning → card preview (tri-state
// section checkboxes, expandable item rows) → confirm sheet → executing →
// hero result. One confirmation pattern for every destructive surface.
// Performance contract: checkbox toggles patch section headers and the footer
// in place — the card list never re-renders wholesale.

import { cancelTask, executePlan, revealInFinder } from "./ipc";
import { esc, humanKb } from "./format";
import { mountParticles, type ParticlePalette } from "./particles";
import { t } from "./i18n";
import type { ExecItem, ExecReport, PlanSection, PlanSummary } from "./types";

export interface FlowOptions {
  title: string;
  /** Kick off the scan; receives a progress-label setter. A view may return
   * several plans (clean + purge + installer) — each keeps its own backend
   * plan/execute funnel; the flow only merges their sections for display. */
  plan: (
    taskId: string,
    onLabel: (label: string) => void,
  ) => Promise<PlanSummary | PlanSummary[]>;
  /** Extra cancel hook for plans scanning under additional task ids. */
  onCancel?: () => void;
  /** Verb shown on buttons and the confirm sheet. */
  verb: string;
  /** Optional hint line under the header (blocked-apps notice, blockers). */
  subtitleHtml?: () => string;
  helperAvailable: boolean;
  /** Particle-field hero in this palette: an orbiting scan vortex while
   * scanning, absorbed-into-center on the result. */
  particles?: ParticlePalette;
  /** Home dir for ~-abbreviated paths in item rows. */
  home?: string;
  /** Clean-only: route deletes to Trash instead of permanent removal. */
  trashOverride?: boolean;
}

/** Icons for the known section keys; unknown sections get the folder. */
const SECTION_ICONS: Record<string, string> = {
  app_cache: "📱",
  logs: "📄",
  dev: "🔨",
  ai: "🧠",
  browser: "🧭",
  design: "🎨",
  im: "💬",
  system: "🔒",
  "Installer artifacts": "📦",
};

/** Render the flow into a container (fresh instance per mount). */
export function renderFlow(container: HTMLElement, taskId: string, opts: FlowOptions): void {
  container.innerHTML = `
    <div class="hero" id="flow-hero">
      <div class="big" style="font-size:22px">${t("flow.scanning")}…</div>
      <div class="sub" id="scan-label"></div>
      <button class="cta" id="cancel-scan">${t("flow.cancel")}</button>
    </div>
    <div class="content-narrow" id="flow-body"></div>`;

  const body = container.querySelector<HTMLElement>("#flow-body")!;
  const hero = container.querySelector<HTMLElement>("#flow-hero")!;
  if (opts.particles) mountParticles(hero, "scan", opts.particles);
  container.querySelector("#cancel-scan")!.addEventListener("click", () => {
    void cancelTask(taskId);
    opts.onCancel?.();
  });

  opts
    .plan(taskId, (label) => {
      const el = container.querySelector<HTMLElement>("#scan-label");
      if (el) el.textContent = label;
    })
    .then((result) => {
      hero.remove();
      renderPreview(body, Array.isArray(result) ? result : [result], opts);
    })
    .catch((e) => {
      hero.innerHTML = `<div class="placeholder">${esc(String(e?.message ?? e))}</div>`;
    });
}

/** Section title/desc from i18n keys, falling back to the raw title. */
function sectionMeta(title: string): { name: string; desc: string; icon: string } {
  const name = t(`sec.${title}.title`);
  if (name !== `sec.${title}.title`) {
    return {
      name,
      desc: t(`sec.${title}.desc`),
      icon: SECTION_ICONS[title] ?? "📁",
    };
  }
  return { name: title, desc: "", icon: "📁" };
}

/** Abbreviate the home prefix to ~ for item rows. */
function tildify(path: string, home?: string): string {
  if (home && path.startsWith(home)) return `~${path.slice(home.length)}`;
  return path;
}

/** Display name for an item: its final path component. */
function displayName(path: string): string {
  return path.replace(/\/$/, "").split("/").pop() ?? path;
}

/** Stage 2: category cards with tri-state checkboxes and expandable items. */
function renderPreview(body: HTMLElement, summaries: PlanSummary[], opts: FlowOptions): void {
  if (summaries.reduce((n, s) => n + s.count, 0) === 0) {
    body.innerHTML = `
      <div class="hero">
        <div class="sub">${t("flow.nothing")}</div>
      </div>`;
    if (opts.particles) mountParticles(body.querySelector<HTMLElement>(".hero")!, "idle", opts.particles);
    return;
  }

  // Flattened sections across all plans; planOf[idx] names the owning plan
  // (item ids are only unique per plan, so grouping stays plan-scoped).
  const sections = summaries.flatMap((s) => s.sections);
  const planOf = summaries.flatMap((s) => s.sections.map(() => s.plan_id));

  // Selection state: per section, the set of checked item ids.
  const checked: Set<string>[] = sections.map((section) =>
    new Set(
      section.items
        .filter((i) => !(i.scope === "system" && !opts.helperAvailable))
        .map((i) => i.id),
    ),
  );

  const cards = sections
    .map((section, idx) => {
      const meta = sectionMeta(section.title);
      return `<div class="clean-card" data-card="${idx}">
        <div class="card-row">
          <input type="checkbox" class="sec-check" data-seccheck="${idx}" />
          <div class="sec-icon">${meta.icon}</div>
          <div class="sec-text">
            <div class="sec-title">${esc(meta.name)}
              <span class="muted" data-seccount="${idx}"></span></div>
            ${meta.desc ? `<div class="sec-desc muted">${esc(meta.desc)}</div>` : ""}
          </div>
          <div class="spacer"></div>
          <div class="sec-size" data-secsize="${idx}"></div>
          <button class="chev" data-chev="${idx}">▾</button>
        </div>
        <div class="sec-items" data-items="${idx}" hidden></div>
      </div>`;
    })
    .join("");

  body.innerHTML = `
    <div class="preview-head">
      <div>
        <h1 style="font-size:22px;margin:0">${esc(opts.title)}</h1>
        ${opts.subtitleHtml?.() ?? ""}
      </div>
      <button class="chev" id="close-preview" title="${t("flow.cancel")}">✕</button>
    </div>
    ${cards}
    <div class="footer-bar">
      <span style="font-family:var(--font-mono);font-size:12px" id="foot-count"></span>
      <a href="#" id="sel-all" style="font-size:12px">${t("prev.selectall")}</a>
      <span class="muted">·</span>
      <a href="#" id="sel-none" style="font-size:12px">${t("prev.clear")}</a>
      <button class="big-btn" id="go"></button>
    </div>`;

  body.querySelector("#close-preview")!.addEventListener("click", () => {
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  });

  /** Patch one section header (count, sizes, tri-state) in place. */
  const patchSection = (idx: number) => {
    const section = sections[idx];
    const set = checked[idx];
    const selKb = section.items.reduce(
      (sum, i) => sum + (set.has(i.id) ? (i.size_kb ?? 0) : 0),
      0,
    );
    body.querySelector(`[data-seccount="${idx}"]`)!.textContent =
      `${t("prev.selected")} ${set.size}/${section.items.length}`;
    const sizeEl = body.querySelector<HTMLElement>(`[data-secsize="${idx}"]`)!;
    sizeEl.textContent =
      set.size === section.items.length
        ? humanKb(section.total_kb)
        : `${humanKb(selKb)} / ${humanKb(section.total_kb)}`;
    const cb = body.querySelector<HTMLInputElement>(`[data-seccheck="${idx}"]`)!;
    cb.checked = set.size === section.items.length && set.size > 0;
    cb.indeterminate = set.size > 0 && set.size < section.items.length;
  };

  /** Patch the footer totals in place. */
  const patchFooter = () => {
    let count = 0;
    let kb = 0;
    sections.forEach((section, idx) => {
      const set = checked[idx];
      count += set.size;
      for (const item of section.items) {
        if (set.has(item.id)) kb += item.size_kb ?? 0;
      }
    });
    body.querySelector("#foot-count")!.textContent =
      `${t("prev.selected")} ${count} ${t("prev.items")}`;
    const go = body.querySelector<HTMLButtonElement>("#go")!;
    go.textContent = `${opts.verb} · ${humanKb(kb)}`;
    go.disabled = count === 0;
  };

  const patchAll = () => {
    sections.forEach((_, idx) => patchSection(idx));
    patchFooter();
  };
  patchAll();

  /** Render the expandable item rows of one section (once, on first expand). */
  const renderItems = (idx: number) => {
    const holder = body.querySelector<HTMLElement>(`[data-items="${idx}"]`)!;
    if (holder.dataset.rendered === "1") {
      syncItemChecks(idx);
      return;
    }
    holder.dataset.rendered = "1";
    const section = sections[idx];
    holder.innerHTML = section.items
      .map((item) => {
        const gated = item.scope === "system" && !opts.helperAvailable;
        return `<div class="item-row">
          <input type="checkbox" data-item="${esc(item.id)}" data-sec="${idx}"
            ${checked[idx].has(item.id) ? "checked" : ""} ${gated ? "disabled" : ""} />
          <div class="item-text">
            <div class="iname">${esc(displayName(item.path))}</div>
            <div class="ipath muted" title="${esc(item.path)}">${esc(tildify(item.path, opts.home))}</div>
          </div>
          <div class="spacer"></div>
          ${item.item_count > 1 ? `<span class="muted icount">${item.item_count} ${t("prev.items")}</span>` : ""}
          <span class="isize">${item.size_kb === null ? "?" : humanKb(item.size_kb)}</span>
          ${gated ? `<span class="badge warn">${t("flow.admin")}</span>` : ""}
          <button class="chev reveal" data-reveal="${esc(item.path)}" title="Finder">📂</button>
        </div>`;
      })
      .join("");

    holder.querySelectorAll<HTMLInputElement>("input[data-item]").forEach((cb) => {
      cb.addEventListener("change", () => {
        const set = checked[Number(cb.dataset.sec)];
        if (cb.checked) set.add(cb.dataset.item!);
        else set.delete(cb.dataset.item!);
        patchSection(Number(cb.dataset.sec));
        patchFooter();
      });
    });
    holder.querySelectorAll<HTMLButtonElement>("button[data-reveal]").forEach((btn) => {
      btn.addEventListener("click", () => void revealInFinder(btn.dataset.reveal!));
    });
  };

  /** Sync rendered item checkboxes to the state after bulk toggles. */
  const syncItemChecks = (idx: number) => {
    body
      .querySelectorAll<HTMLInputElement>(`input[data-item][data-sec="${idx}"]`)
      .forEach((cb) => {
        if (!cb.disabled) cb.checked = checked[idx].has(cb.dataset.item!);
      });
  };

  // Wire section checkboxes, chevrons, and bulk links.
  body.querySelectorAll<HTMLInputElement>("input[data-seccheck]").forEach((cb) => {
    cb.addEventListener("change", () => {
      const idx = Number(cb.dataset.seccheck);
      const section = sections[idx];
      checked[idx] = cb.checked
        ? new Set(
            section.items
              .filter((i) => !(i.scope === "system" && !opts.helperAvailable))
              .map((i) => i.id),
          )
        : new Set();
      syncItemChecks(idx);
      patchSection(idx);
      patchFooter();
    });
  });
  body.querySelectorAll<HTMLButtonElement>("button[data-chev]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const idx = Number(btn.dataset.chev);
      const holder = body.querySelector<HTMLElement>(`[data-items="${idx}"]`)!;
      if (holder.hidden) {
        holder.hidden = false;
        btn.textContent = "▴";
        renderItems(idx);
      } else {
        holder.hidden = true;
        btn.textContent = "▾";
      }
    });
  });
  body.querySelector("#sel-all")!.addEventListener("click", (ev) => {
    ev.preventDefault();
    sections.forEach((section, idx) => {
      checked[idx] = new Set(
        section.items
          .filter((i) => !(i.scope === "system" && !opts.helperAvailable))
          .map((i) => i.id),
      );
      syncItemChecks(idx);
    });
    patchAll();
  });
  body.querySelector("#sel-none")!.addEventListener("click", (ev) => {
    ev.preventDefault();
    sections.forEach((_, idx) => {
      checked[idx] = new Set();
      syncItemChecks(idx);
    });
    patchAll();
  });

  body.querySelector("#go")!.addEventListener("click", () => {
    // Group the checked ids back by owning plan: each plan executes through
    // its own two-phase funnel, the flow only aggregates the reports.
    const byPlan = new Map<string, string[]>();
    let count = 0;
    let kb = 0;
    sections.forEach((section, idx) => {
      const set = checked[idx];
      if (set.size === 0) return;
      count += set.size;
      for (const item of section.items) {
        if (set.has(item.id)) kb += item.size_kb ?? 0;
      }
      byPlan.set(planOf[idx], [...(byPlan.get(planOf[idx]) ?? []), ...set]);
    });
    const jobs = [...byPlan.entries()].map(([planId, selection]) => ({ planId, selection }));
    confirmSheet(count, kb, opts.verb, () => runExecution(body, jobs, opts));
  });
}

/** The one confirm sheet, shared by every destructive surface. */
export function confirmSheet(
  count: number,
  totalKb: number,
  verb: string,
  onConfirm: () => void,
): void {
  const sheet = document.createElement("div");
  sheet.className = "confirm-backdrop";
  sheet.innerHTML = `
    <div class="confirm-sheet">
      <h2>${esc(t("flow.confirm.title", { n: count }))}</h2>
      <p class="muted">${esc(t("flow.confirm.sub", { size: humanKb(totalKb) }))}</p>
      <div class="confirm-actions">
        <button id="confirm-cancel">${t("flow.cancel")}</button>
        <button id="confirm-go" class="danger">${esc(verb)}</button>
      </div>
    </div>`;
  document.body.appendChild(sheet);
  sheet.querySelector("#confirm-cancel")!.addEventListener("click", () => sheet.remove());
  sheet.querySelector("#confirm-go")!.addEventListener("click", () => {
    sheet.remove();
    onConfirm();
  });
}

/** One executable unit: a stored plan id plus its checked selection. */
export interface ExecJob {
  planId: string;
  selection: string[];
}

/** Stage 4: live execution feed, then the hero result. Plans execute
 * strictly one after another (each through its own two-phase funnel); the
 * result hero shows the aggregated report. */
export function runExecution(
  body: HTMLElement,
  jobs: ExecJob[],
  opts: Pick<FlowOptions, "particles" | "trashOverride">,
): void {
  body.innerHTML = `
    <div class="muted" id="exec-line" style="text-align:center;margin:10px 0">${t("flow.working")}…</div>
    <div class="panel"><table><tbody id="exec-feed"></tbody></table></div>`;
  const feed = body.querySelector<HTMLElement>("#exec-feed")!;
  const line = body.querySelector<HTMLElement>("#exec-line")!;
  const total = jobs.reduce((n, j) => n + j.selection.length, 0);
  let done = 0;

  const onItem = (item: ExecItem) => {
    done += 1;
    line.textContent = `${t("flow.working")}… ${done}/${total}`;
    const row = document.createElement("tr");
    row.innerHTML = `
      <td><span class="badge ${badgeFor(item.outcome)}">${esc(item.outcome)}</span></td>
      <td title="${esc(item.path)}">${esc(item.path)}</td>
      <td class="num">${item.size_kb === null ? "" : humanKb(item.size_kb)}</td>
      <td class="muted">${esc(item.error ?? "")}</td>`;
    feed.prepend(row);
  };

  (async () => {
    let freedKb = 0;
    let skipped = 0;
    let failed = 0;
    for (const job of jobs) {
      const report: ExecReport = await executePlan(
        job.planId,
        job.selection,
        false,
        onItem,
        opts.trashOverride ?? null,
      );
      freedKb += report.total_freed_kb;
      skipped += report.skipped;
      failed += report.failed;
    }
    return { freedKb, skipped, failed };
  })()
    .then(({ freedKb, skipped, failed }) => {
      body.innerHTML = `
        <div class="hero">
          <div class="big">${humanKb(freedKb)}</div>
          <div class="sub">${t("flow.freed")} · ${skipped} ${t("flow.skipped")} · ${failed} ${t("flow.failed")}</div>
          <button class="cta" id="again">${t("clean.again")}</button>
        </div>`;
      // Post-clean effect: the remaining junk particles get absorbed into
      // the center bloom behind the freed-space number.
      if (opts.particles)
        mountParticles(body.querySelector<HTMLElement>(".hero")!, "reclaimed", opts.particles);
      body.querySelector("#again")!.addEventListener("click", () => {
        window.dispatchEvent(new HashChangeEvent("hashchange"));
      });
    })
    .catch((e) => {
      line.textContent = String(e?.message ?? e);
    });
}

/** Badge color for an execution outcome. */
export function badgeFor(outcome: string): string {
  if (outcome === "trashed" || outcome === "removed" || outcome === "dry-run") return "ok";
  if (outcome === "failed") return "danger";
  return "warn";
}

/** Re-exported for views composing their own headers. */
export type { PlanSection };
