// Optimize view: azure particle hero with a one-click run button — no task
// list. Tasks run SEQUENTIALLY; progress is a single ticker line pinned to
// the hero's bottom edge so the particle animation stays unobstructed.
// Admin tasks are skipped up front when the privileged helper is unavailable.

import { appMeta, listOptimizeTasks, runOptimize } from "../ipc";
import { mountParticles } from "../particles";
import { t } from "../i18n";
import type { View } from "../router";
import type { OptimizeTask, TaskResult } from "../types";

/** Session cache: the task catalog is static and the helper state rarely
 * changes, so re-entering the tab renders instantly instead of re-querying. */
let cached: { tasks: OptimizeTask[]; helper: boolean } | null = null;

export const optimize: View = {
  async mount(container) {
    if (!cached) {
      const [tasks, meta] = await Promise.all([listOptimizeTasks(), appMeta()]);
      cached = { tasks, helper: meta.helper_available };
    }
    renderIdle(container, cached.tasks, cached.helper);
  },
};

/** Idle stage: particle field + run button only; no task list by default. */
function renderIdle(container: HTMLElement, tasks: OptimizeTask[], helper: boolean): void {
  // Same gating rule the old checklist applied per checkbox: admin tasks are
  // excluded entirely when the privileged helper is unavailable.
  const runnable = tasks.filter((task) => !(task.requires_admin && !helper));
  const gatedCount = tasks.length - runnable.length;

  container.innerHTML = `
    <div class="hero">
      <div class="big" style="font-size:24px">${t("opt.title")}</div>
      <div class="sub">${t("opt.sub")}</div>
      ${gatedCount > 0 ? `<div class="muted" style="margin-top:8px">${t("opt.helper")}</div>` : ""}
      <button class="cta primary" id="run">${t("opt.run")}</button>
    </div>`;
  mountParticles(container.querySelector<HTMLElement>(".hero")!, "idle", "azure");

  container.querySelector("#run")!.addEventListener("click", () => {
    if (runnable.length === 0) return;
    void renderRunning(container, runnable);
  });
}

/** Running stage: sequential execution with a single bottom-pinned ticker line. */
async function renderRunning(container: HTMLElement, tasks: OptimizeTask[]): Promise<void> {
  container.innerHTML = `
    <div class="hero">
      <div class="big" id="title" style="font-size:24px">${t("opt.running")}</div>
      <div class="sub" id="sub"></div>
      <div class="opt-ticker" id="ticker"></div>
    </div>`;
  const hero = container.querySelector<HTMLElement>(".hero")!;
  const title = container.querySelector<HTMLElement>("#title")!;
  const sub = container.querySelector<HTMLElement>("#sub")!;
  const ticker = container.querySelector<HTMLElement>("#ticker")!;
  // Continuous intake (no vortex ring): particles fall into the center and
  // respawn at the rim for as long as the tasks run.
  mountParticles(hero, "absorb", "azure");

  const results: TaskResult[] = [];
  for (let i = 0; i < tasks.length; i++) {
    const task = tasks[i];
    tick(ticker, `● ${task.title} · ${i + 1}/${tasks.length}`);
    // One task per IPC call keeps the ticker honest about what is running.
    const [result] = await runOptimize([task.id]);
    results.push(result ?? { id: task.id, outcome: "failed", output: "" });
  }

  // Compact summary: real execution errors are "failed"; every refusal or
  // skip (admin gate, apps running, probe unknown, unavailable…) counts as
  // skipped so the closing line stays one glance long.
  const okCount = results.filter((r) => ["ok", "unchanged"].includes(r.outcome)).length;
  const failCount = results.filter((r) => r.outcome === "failed").length;
  const skipCount = results.length - okCount - failCount;
  const parts = [`${okCount} ${t("opt.sum.ok")}`];
  if (skipCount > 0) parts.push(`${skipCount} ${t("opt.sum.skip")}`);
  if (failCount > 0) parts.push(`${failCount} ${t("opt.sum.fail")}`);

  ticker.remove();
  title.textContent = t("opt.done");
  sub.textContent = parts.join(" · ");
  // Swap the endless intake for the closing absorb-into-bloom; the running
  // canvas must go first or both would keep animating stacked on top of
  // each other.
  hero.querySelector("canvas.particles")?.remove();
  mountParticles(hero, "reclaimed", "azure");
  const back = document.createElement("button");
  back.className = "cta";
  back.textContent = t("clean.again");
  back.addEventListener("click", () => {
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  });
  hero.append(back);
}

/** Swaps the ticker text and replays its slide-up entrance animation. */
function tick(el: HTMLElement, text: string): void {
  el.textContent = text;
  el.classList.remove("tick");
  // Force a reflow so removing and re-adding the class restarts the animation.
  void el.offsetWidth;
  el.classList.add("tick");
}
