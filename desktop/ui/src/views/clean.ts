// Clean view: particle-field hero → one merged scan (system clean + project
// build artifacts + installer leftovers) → sectioned preview → confirm →
// execute → freed-space hero with the absorbed-particles effect. Purge and
// installer keep their own backend plans and two-phase funnels; this surface
// only merges their sections into one flow.

import {
  appMeta,
  cancelTask,
  newTaskId,
  planClean,
  planInstaller,
  planPurge,
} from "../ipc";
import { renderFlow } from "../flow";
import { mountParticles } from "../particles";
import { cleanTrashMode } from "../prefs";
import { esc, humanKb } from "../format";
import { t } from "../i18n";
import type { View } from "../router";
import type { BlockedCaches, PlanSummary, ProjectReport } from "../types";

export const clean: View = {
  mount(container) {
    container.innerHTML = `
      <div class="hero">
        <div class="big" style="font-size:24px">${t("clean.ready")}</div>
        <div class="sub">${t("clean.ready.sub")}</div>
        <button class="cta primary" id="scan">${t("clean.scan")}</button>
      </div>`;
    // Default hero: the floating junk-particle field (replaces the Earth).
    mountParticles(container.querySelector<HTMLElement>(".hero")!, "idle", "ember");
    container.querySelector("#scan")!.addEventListener("click", async () => {
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
