// Clean view: particle-field hero → one merged scan (system clean + project
// build artifacts + installer leftovers) → sectioned preview → confirm →
// execute → freed-space hero with the absorbed-particles effect. Purge and
// installer keep their own backend plans and two-phase funnels; this surface
// only merges their sections into one flow.

import {
  appMeta,
  cancelTask,
  celestialCatalog,
  newTaskId,
  planClean,
  planInstaller,
  planPurge,
} from "../ipc";
import { renderFlow } from "../flow";
import {
  mountParticles,
  type CelestialPreset,
  type ParticleController,
} from "../particles";
import { cleanTrashMode } from "../prefs";
import { esc, humanKb } from "../format";
import { lang, t } from "../i18n";
import type { View } from "../router";
import type { BlockedCaches, PlanSummary, ProjectReport } from "../types";

export const clean: View = {
  mount(container) {
    container.innerHTML = `
      <div class="hero">
        <button class="explore-toggle" id="explore-toggle" aria-pressed="false">
          <span aria-hidden="true">✦</span>
          <span id="explore-toggle-label">${t("clean.explore")}</span>
        </button>
        <section class="explore-card" id="explore-card" aria-live="polite" hidden>
          <div class="explore-presets" role="group" aria-label="${t("clean.explore.presets")}">
            <button class="active" data-preset="all">${t("clean.explore.all")}</button>
            <button data-preset="solar-system">${t("clean.explore.solar")}</button>
            <button data-preset="earth-moon">${t("clean.explore.earthMoon")}</button>
          </div>
          <div class="explore-card-head">
            <span id="explore-progress">${t("clean.explore.loading")}</span>
            <span class="explore-new" id="explore-new" hidden>${t("clean.explore.new")}</span>
          </div>
          <div class="explore-visual" id="explore-visual" data-kind="planet" aria-hidden="true">
            <div class="explore-visual-orbit"></div>
            <div class="explore-visual-body" id="explore-visual-body"></div>
            <img class="explore-visual-image" id="explore-visual-image" alt="" hidden />
            <span id="explore-visual-caption">${t("clean.explore.illustration")}</span>
          </div>
          <div class="explore-name" id="explore-name">${t("clean.explore.prompt")}</div>
          <div class="explore-meta" id="explore-meta">${t("clean.explore.hint")}</div>
          <div class="explore-size" id="explore-size">${t("clean.explore.scale")}</div>
          <p class="explore-summary" id="explore-summary"></p>
          <div class="explore-source" id="explore-source"></div>
          <button class="explore-reset" id="explore-reset">${t("clean.explore.reset")}</button>
        </section>
        <div class="big" style="font-size:24px">${t("clean.ready")}</div>
        <div class="sub">${t("clean.ready.sub")}</div>
        <button class="cta primary" id="scan">${t("clean.scan")}</button>
      </div>`;
    const hero = container.querySelector<HTMLElement>(".hero")!;
    const toggle = container.querySelector<HTMLButtonElement>("#explore-toggle")!;
    const toggleLabel = container.querySelector<HTMLElement>("#explore-toggle-label")!;
    const card = container.querySelector<HTMLElement>("#explore-card")!;
    const readyTitle = container.querySelector<HTMLElement>(".hero > .big")!;
    const readySubtitle = container.querySelector<HTMLElement>(".hero > .sub")!;
    const scanButton = container.querySelector<HTMLButtonElement>("#scan")!;
    const progress = container.querySelector<HTMLElement>("#explore-progress")!;
    const discoveryName = container.querySelector<HTMLElement>("#explore-name")!;
    const discoveryMeta = container.querySelector<HTMLElement>("#explore-meta")!;
    const discoverySize = container.querySelector<HTMLElement>("#explore-size")!;
    const discoverySummary = container.querySelector<HTMLElement>("#explore-summary")!;
    const catalogSource = container.querySelector<HTMLElement>("#explore-source")!;
    const newBadge = container.querySelector<HTMLElement>("#explore-new")!;
    const preview = container.querySelector<HTMLElement>("#explore-visual")!;
    const previewBody = container.querySelector<HTMLElement>("#explore-visual-body")!;
    const previewImage = container.querySelector<HTMLImageElement>("#explore-visual-image")!;
    const previewCaption = container.querySelector<HTMLElement>("#explore-visual-caption")!;
    const presetButtons = Array.from(
      container.querySelectorAll<HTMLButtonElement>("[data-preset]"),
    );
    let exploring = false;
    let particles: ParticleController | null = null;

    /** Keep the optional entertainment layer separate from the primary clean
     * action. Leaving explorer mode restores the original quiet hero. */
    const setExploring = (active: boolean) => {
      exploring = active;
      hero.classList.toggle("exploring", active);
      card.hidden = !active;
      toggle.setAttribute("aria-pressed", String(active));
      toggleLabel.textContent = t(active ? "clean.explore.exit" : "clean.explore");
      readyTitle.setAttribute("aria-hidden", String(active));
      readySubtitle.setAttribute("aria-hidden", String(active));
      scanButton.disabled = active;
      particles?.setExploring(active);
      if (!active) toggle.focus({ preventScroll: true });
    };

    particles = mountParticles(hero, "idle", "ember", {
      interactive: true,
      targets: [],
      ariaLabel: t("clean.explore.aria"),
      onExit: () => setExploring(false),
      onDiscovery: ({ target, discovered, total, isNew }) => {
        progress.textContent = t("clean.explore.progress", { n: discovered, total });
        discoveryName.textContent = localizedCelestialName(target.id, target.name);
        const type = t(`explore.type.${target.bodyType}`);
        const distance =
          target.distanceLy === null
            ? t("clean.explore.distanceUnknown")
            : target.distanceLy < 0.000_001
            ? t("clean.explore.local")
            : t("clean.explore.distance", { distance: formatDistance(target.distanceLy) });
        discoveryMeta.textContent = `${type} · ${distance}`;
        discoverySize.textContent =
          target.radiusKm === null
            ? t("clean.explore.radiusUnknown")
            : t("clean.explore.radius", { radius: formatRadius(target.radiusKm) });
        discoverySummary.textContent = lang() === "zh" ? target.summaryZh : target.summaryEn;
        catalogSource.textContent = t("clean.explore.objectSource", {
          source: target.sourceUrl.includes("science.nasa.gov")
            ? "NASA Solar System"
            : "NASA Exoplanet Archive",
        });
        renderCelestialPreview(preview, previewBody, previewImage, previewCaption, target);
        newBadge.hidden = !isNew;
      },
    });

    // Catalog loading is independent of the cleaning flow. SQLite opens and
    // any network refresh run on a blocking backend worker, so the main action
    // and the ambient animation remain responsive.
    void celestialCatalog()
      .then((catalog) => {
        particles?.setTargets(catalog.objects);
        progress.textContent = t("clean.explore.progress", {
          n: 0,
          total: catalog.objects.length,
        });
        catalogSource.textContent = catalog.archiveComplete
          ? t("clean.explore.catalog", { source: catalog.sourceName })
          : t("clean.explore.offlineCatalog");
        if (catalog.warning) catalogSource.title = catalog.warning;
      })
      .catch(() => {
        progress.textContent = t("clean.explore.loadFailed");
        catalogSource.textContent = t("clean.explore.loadFailedHint");
      });
    toggle.addEventListener("click", () => setExploring(!exploring));
    const setPreset = (preset: CelestialPreset) => {
      particles?.focusPreset(preset);
      for (const button of presetButtons) {
        button.classList.toggle("active", button.dataset.preset === preset);
      }
    };
    for (const button of presetButtons) {
      button.addEventListener("click", () => setPreset(button.dataset.preset as CelestialPreset));
    }
    container
      .querySelector("#explore-reset")!
      .addEventListener("click", () => setPreset("all"));

    scanButton.addEventListener("click", async () => {
      const meta = await appMeta();
      let blocked: BlockedCaches | null = null;
      let projects: ProjectReport[] = [];
      // Purge/installer scan under their own task ids so cancel reaches all
      // three backend scans, not only the clean one.
      const purgeTask = newTaskId();
      const installerTask = newTaskId();
      renderFlow(container, newTaskId(), {
        title: t("prev.header"),
        verb: t("clean.go"),
        helperAvailable: meta.helper_available,
        particles: "ember",
        home: meta.home,
        trashOverride: cleanTrashMode(),
        onCancel: () => {
          void cancelTask(purgeTask);
          void cancelTask(installerTask);
        },
        subtitleHtml: () => {
          const parts: string[] = [];
          if (blocked && blocked.count > 0) {
            const hint = t("prev.blocked", {
              apps: blocked.owners.join("、"),
              size: humanKb(blocked.total_kb),
              n: blocked.count,
            });
            parts.push(`<div class="muted" style="margin-top:6px">${hint}</div>`);
          }
          // Project git blockers stay report-only badges, never a verdict.
          const flagged = projects.filter((p) => p.blockers.length > 0);
          if (flagged.length > 0) {
            parts.push(
              `<div class="note-list">${flagged
                .map(
                  (p) =>
                    `<div class="badge warn" title="${esc(p.root)}">${esc(shorten(p.root))}: ${esc(
                      p.blockers.join(", "),
                    )}</div>`,
                )
                .join(" ")}</div>`,
            );
          }
          return parts.join("");
        },
        plan: async (taskId, onLabel) => {
          // All three scans run in parallel. The clean scan is the primary:
          // its failure (including cancel) aborts the flow; a purge or
          // installer failure degrades to a preview without those sections.
          const [c, p, i] = await Promise.allSettled([
            planClean(taskId, (e) => onLabel(`${e.label} (${e.count})`)),
            planPurge(purgeTask, (e) => onLabel(shorten(e.label))),
            planInstaller(installerTask),
          ]);
          if (c.status === "rejected") throw c.reason;
          blocked = c.value.blocked;
          const summaries: PlanSummary[] = [c.value.summary];
          if (p.status === "fulfilled") {
            projects = p.value.projects;
            summaries.push(p.value.summary);
          }
          if (i.status === "fulfilled") summaries.push(i.value);
          return summaries;
        },
      });
    });
  },
};

/** Shorten a project path for progress/badge display. */
function shorten(path: string): string {
  return path.split("/").slice(-2).join("/");
}

/** Compact scientific distances without rounding nearby objects to zero. */
function formatDistance(lightYears: number): string {
  if (lightYears < 0.01) return lightYears.toExponential(2);
  if (lightYears < 100) return lightYears.toFixed(2);
  return Math.round(lightYears).toLocaleString();
}

/** Physical radius shown in km; marker rendering applies the documented
 * logarithmic scale separately so this factual value is never distorted. */
function formatRadius(radiusKm: number): string {
  return Math.round(radiusKm).toLocaleString();
}

/** Solar System common names are localized; catalog designations remain
 * untouched because identifiers such as K2-9 or HD 2039 are proper names. */
function localizedCelestialName(id: string, fallback: string): string {
  const solarIds = new Set([
    "sun",
    "mercury",
    "venus",
    "earth",
    "moon",
    "mars",
    "jupiter",
    "saturn",
    "uranus",
    "neptune",
    "ceres",
    "pluto",
    "eris",
  ]);
  return solarIds.has(id) ? t(`explore.name.${id}`) : fallback;
}

const REAL_TEXTURES: Readonly<Record<string, string>> = {
  earth: "/planet-earth.jpg",
  mars: "/planet-mars.jpg",
  jupiter: "/planet-jupiter.jpg",
};

/** Render an honest visual companion to the catalog prose. Three bundled NASA
 * textures are identified as real textures; every other body gets a stable,
 * explicitly labelled data illustration derived from its type and id. */
function renderCelestialPreview(
  preview: HTMLElement,
  body: HTMLElement,
  image: HTMLImageElement,
  caption: HTMLElement,
  target: import("../explore").CelestialTarget,
): void {
  const texture = REAL_TEXTURES[target.id];
  const hue = stableHue(target.id);
  preview.dataset.kind = target.bodyType;
  preview.style.setProperty("--preview-hue", String(hue));
  image.hidden = !texture;
  body.hidden = Boolean(texture);
  if (texture) {
    image.src = texture;
    caption.textContent = t("clean.explore.realTexture");
  } else {
    image.removeAttribute("src");
    caption.textContent = t("clean.explore.illustration");
  }
}

/** FNV-1a produces a stable color identity without persisting presentation
 * fields in the scientific catalog. */
function stableHue(id: string): number {
  let hash = 2_166_136_261;
  for (const char of id) {
    hash ^= char.charCodeAt(0);
    hash = Math.imul(hash, 16_777_619);
  }
  return Math.abs(hash) % 360;
}
