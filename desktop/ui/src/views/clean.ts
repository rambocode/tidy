// Clean view (newspaper skin): a front-page hero — issue kicker, serif
// headline, lede, framed CTA and a monospaced statline drawn from the local
// ledger — then one merged scan (system clean + project build artifacts +
// installer leftovers + docker) → ledger preview → confirm → execute →
// reclaimed-space page. Purge and installer keep their own backend plans and
// two-phase funnels; this surface only merges their sections into one flow.

import {
  appMeta,
  cancelTask,
  newTaskId,
  planClean,
  planInstaller,
  planPurge,
  planDocker,
  executeDocker,
} from "../ipc";
import { renderFlow } from "../flow";
import { renderFrontPage } from "../frontpage";
import { cleanTrashMode, dockerIdleMonths, purgeIdleDays } from "../prefs";
import { esc, humanKb } from "../format";
import { daysSince, lastReclaim, reclaimLog, totalFreedKb } from "../ledger";
import { dateline, relativeDay, t } from "../i18n";
import type { View } from "../router";
import type { BlockedCaches, PlanSummary, ProjectReport,
  DockerImage,
} from "../types";

export const clean: View = {
  mount(container) {
    const previous = lastReclaim("clean");
    // The "issue number" is simply how many clean runs this machine has seen,
    // so the masthead conceit stays factual instead of decorative.
    const issue = reclaimLog().filter((r) => r.kind === "clean").length + 1;
    const total = totalFreedKb();

    const scanButton = renderFrontPage(container, {
      kicker: t("clean.issue", { n: issue }),
      strapline: t("clean.strapline"),
      // The plate number is the issue number, so the numeral on the page and
      // the one in the kicker are always the same fact.
      watermark: String(issue),
      headlineHtml: t("clean.headline"),
      desk: t("clean.desk"),
      dateline: dateline(),
      action: t("clean.scan"),
      actionNote: t("clean.scanNote"),
      noteBody: t("clean.standfirst"),
      stats: [
        {
          label: t("clean.stat.last"),
          value: previous ? relativeDay(daysSince(previous.at)) : "—",
        },
        {
          label: t("clean.stat.reclaimed"),
          value: previous ? humanKb(previous.freedKb) : "—",
        },
        { label: t("clean.stat.total"), value: total > 0 ? humanKb(total) : "—" },
      ],
    });


    scanButton.addEventListener("click", async () => {
      const meta = await appMeta();
      let blocked: BlockedCaches | null = null;
      let projects: ProjectReport[] = [];
      let images: DockerImage[] = [];
      let dockerPlanId = "";
      const idleDays = purgeIdleDays();
      const idleMonths = dockerIdleMonths();
      // Purge/installer/docker scan under their own task ids so cancel
      // reaches all backend scans, not only the clean one.
      const purgeTask = newTaskId();
      const installerTask = newTaskId();
      const dockerTask = newTaskId();
      renderFlow(container, newTaskId(), {
        title: t("prev.header"),
        verb: t("clean.go"),
        helperAvailable: meta.helper_available,
        home: meta.home,
        ledger: "clean",
        trashOverride: cleanTrashMode(),
        onCancel: () => {
          void cancelTask(purgeTask);
          void cancelTask(installerTask);
          void cancelTask(dockerTask);
        },
        subtitleHtml: () => {
          const parts: string[] = [];
          if (blocked && blocked.count > 0) {
            const hint = t("prev.blocked", {
              apps: blocked.owners.join("、"),
              size: humanKb(blocked.total_kb),
              n: blocked.count,
            });
            parts.push(`<p class="lede" style="margin-top:6px">${hint}</p>`);
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
        // Default checkbox state comes from the user's thresholds (Settings):
        // project deps idle ≥ N days, Docker images unused ≥ N months (no
        // container uses them; docker has no last-used stamp, so age is
        // creation age). Dangling images and unused build cache are always
        // checked; in-use images never are.
        defaultUnchecked: (item, section) => {
          if (section === "projects") {
            const p = projectOf(projects, item.path);
            return p !== undefined && (p.idle_days === null || p.idle_days < idleDays);
          }
          if (section === "docker") {
            const img = imageOf(images, item.id);
            if (!img) return false;
            if (img.containers > 0) return true;
            if (img.dangling) return false;
            return img.age_days === null || img.age_days < idleMonths * 30;
          }
          return false;
        },
        itemBadge: (item, section) => {
          if (section === "projects") {
            const p = projectOf(projects, item.path);
            if (!p || p.idle_days === null) return "";
            return p.idle_days < idleDays
              ? `<span class="badge warn">${esc(t("prev.proj.recent", { d: p.idle_days }))}</span>`
              : `<span class="badge ok">${esc(t("prev.proj.idle", { d: p.idle_days }))}</span>`;
          }
          if (section === "docker") {
            const img = imageOf(images, item.id);
            if (!img) return "";
            if (img.containers > 0)
              return `<span class="badge warn">${esc(t("prev.docker.inuse", { n: img.containers }))}</span>`;
            if (img.dangling) return `<span class="badge ok">${esc(t("prev.docker.dangling"))}</span>`;
            if (img.age_days === null) return "";
            return `<span class="badge ${img.age_days >= idleMonths * 30 ? "ok" : "warn"}">${esc(
              t("prev.docker.age", { d: img.age_days }),
            )}</span>`;
          }
          return "";
        },
        confirmNote: (selectedItems) => {
          const roots = new Set<string>();
          let kb = 0;
          let recent = 0;
          let dockerCount = 0;
          for (const item of selectedItems) {
            if (imageOf(images, item.id) || item.id === "build-cache") {
              dockerCount += 1;
              continue;
            }
            const p = projectOf(projects, item.path);
            if (!p) continue;
            kb += item.size_kb ?? 0;
            if (!roots.has(p.root) && (p.idle_days === null || p.idle_days < idleDays)) recent += 1;
            roots.add(p.root);
          }
          const notes: string[] = [];
          if (roots.size > 0)
            notes.push(
              t("flow.confirm.projects", { n: roots.size, size: humanKb(kb), m: recent, days: idleDays }),
            );
          if (dockerCount > 0) notes.push(t("flow.confirm.docker", { n: dockerCount }));
          return notes.join(" ");
        },
        execute: (job, onItem) =>
          job.planId === dockerPlanId ? executeDocker(job.planId, job.selection, onItem) : undefined,
        plan: async (taskId, onLabel) => {
          // All four scans run in parallel. The clean scan is the primary:
          // its failure (including cancel) aborts the flow; a purge,
          // installer or docker failure (docker not running is the common
          // case) degrades to a preview without that section.
          const [c, p, i, d] = await Promise.allSettled([
            planClean(taskId, (e) => onLabel(`${e.label} (${e.count})`)),
            planPurge(purgeTask, (e) => onLabel(shorten(e.label))),
            planInstaller(installerTask),
            planDocker(dockerTask),
          ]);
          if (c.status === "rejected") throw c.reason;
          blocked = c.value.blocked;
          const summaries: PlanSummary[] = [c.value.summary];
          if (p.status === "fulfilled") {
            projects = p.value.projects;
            summaries.push(p.value.summary);
          }
          if (i.status === "fulfilled") summaries.push(i.value);
          if (d.status === "fulfilled" && d.value.summary.count > 0) {
            images = d.value.images;
            dockerPlanId = d.value.summary.plan_id;
            summaries.push(d.value.summary);
          }
          return summaries;
        },
      });
    });
  },
};

/** Find the reported project that owns an artifact path (longest root wins
 * so nested packages resolve to the inner project). */
function projectOf(projects: ProjectReport[], path: string): ProjectReport | undefined {
  let best: ProjectReport | undefined;
  for (const p of projects) {
    if (path.startsWith(p.root + "/") && (!best || p.root.length > best.root.length)) best = p;
  }
  return best;
}

/** Docker image row for a plan item id (ids are the full sha256 ids). */
function imageOf(images: DockerImage[], id: string): DockerImage | undefined {
  return images.find((img) => img.id === id);
}

/** Shorten a project path for progress/badge display. */
function shorten(path: string): string {
  return path.split("/").slice(-2).join("/");
}
