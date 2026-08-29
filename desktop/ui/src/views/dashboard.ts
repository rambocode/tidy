// Status view (reference design): 4×2 metric card grid — health with
// hardware chips, CPU with per-core bars, GPU with a rolling sparkline,
// memory with pressure + swap, battery detail, disk gradient bar, network
// area chart, fan RPM with a rolling sparkline (read-only SMC; control would
// need the privileged helper) — and a sortable 50-row process table.
// Refreshes every 2s while mounted only.

import { appIcon, processDetail, revealInFinder, signalProcess, statusSnapshot } from "../ipc";
import { esc, humanBytes, humanUptime } from "../format";
import { t } from "../i18n";
import { formatTemp } from "../prefs";
import type { View } from "../router";
import type { FanStatus, ProcessDetail, ProcessInfo, StatusSnapshot } from "../types";

let timer: number | null = null;

/** Rolling histories for sparklines (persist across renders while mounted). */
const gpuHistory: number[] = [];
const rxHistory: number[] = [];
const txHistory: number[] = [];
const fanHistory: number[] = [];
const HISTORY = 30;

/** Process table sort state. */
let sortCol: "name" | "pid" | "cpu" | "energy" | "mem" = "cpu";
let sortDesc = true;

/** Last snapshot, kept for instant client-side re-sorts on header click. */
let lastSnap: StatusSnapshot | null = null;

/** Process icon cache: app path → data URL (null = known miss). */
const procIcons = new Map<string, string | null>();

/** Single-flight guard for the icon fetch loop: each 2s render used to start
 * its own loop over the still-uncached paths, so slow fetches stacked into
 * unbounded concurrent backend calls. Only one loop runs; later renders pick
 * up whatever is still missing after it finishes. */
let iconLoopRunning = false;

/** Accumulated CPU time as the energy-impact proxy ("2.4h" / "35m" / "12s"). */
function humanCpuTime(ms: number): string {
  const sec = ms / 1000;
  if (sec >= 3600) return `${(sec / 3600).toFixed(1)}h`;
  if (sec >= 60) return `${Math.round(sec / 60)}m`;
  return `${Math.round(sec)}s`;
}

/** Human run time like "4d 12h". */
function humanRuntime(seconds: number): string {
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

/** Push into a rolling buffer. */
function push(buf: number[], v: number): void {
  buf.push(v);
  if (buf.length > HISTORY) buf.shift();
}

/** Health score + the top issue message. */
function health(s: StatusSnapshot): { score: number; message: string } {
  let score = 100;
  const issues: string[] = [];
  const memPct = (s.memory_used_bytes / s.memory_total_bytes) * 100;
  const pressure = s.memory_pressure_percent ?? 0;
  if (s.cpu_usage_percent > 80) {
    score -= 15;
    issues.push("CPU");
  } else if (s.cpu_usage_percent > 60) score -= 5;
  if (pressure > 50 || memPct > 90) {
    score -= 15;
    issues.push(t("st.mem"));
  } else if (memPct > 75) score -= 5;
  const root = s.disks.find((d) => d.mount_point === "/");
  if (root) {
    const freePct = (root.available_bytes / root.total_bytes) * 100;
    if (freePct < 5) {
      score -= 20;
      issues.push(t("st.disk"));
    } else if (freePct < 15) score -= 8;
  }
  const message =
    issues.length === 0 ? t("st.health.good") : `${issues.join(" · ")} ${t("st.health.watch")}`;
  return { score: Math.max(0, score), message };
}

/** Load label from load average vs core count. */
function loadLabel(s: StatusSnapshot): string {
  const ratio = s.load_avg_1m / Math.max(1, s.cpu_count);
  if (ratio < 0.5) return t("st.lowload");
  if (ratio < 0.9) return t("st.midload");
  return t("st.highload");
}

/** Per-core bar chart (inline flex divs). */
function coreBars(cores: number[]): string {
  const bars = cores
    .map((c) => {
      const h = Math.max(3, Math.round((c / 100) * 44));
      return `<div class="core-bar" style="height:${h}px"></div>`;
    })
    .join("");
  return `<div class="core-bars">${bars}</div>`;
}

/** SVG polyline sparkline for a rolling buffer (0-100 scale or auto max). */
function sparkline(buf: number[], color: string, autoScale = false): string {
  if (buf.length < 2) return `<svg class="spark" viewBox="0 0 100 30"></svg>`;
  const max = autoScale ? Math.max(1, ...buf) : 100;
  const points = buf
    .map((v, i) => {
      const x = (i / (HISTORY - 1)) * 100;
      const y = 28 - (Math.min(v, max) / max) * 26;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return `<svg class="spark" viewBox="0 0 100 30" preserveAspectRatio="none">
    <polyline points="${points}" fill="none" stroke="${color}" stroke-width="1.5" /></svg>`;
}

/** Dual-line network chart (rx green, tx blue), auto-scaled together. */
function netChart(): string {
  if (rxHistory.length < 2) return `<svg class="spark" viewBox="0 0 100 30"></svg>`;
  const max = Math.max(1, ...rxHistory, ...txHistory);
  const line = (buf: number[], color: string, fill: boolean) => {
    const pts = buf
      .map((v, i) => `${((i / (HISTORY - 1)) * 100).toFixed(1)},${(28 - (v / max) * 26).toFixed(1)}`)
      .join(" ");
    return fill
      ? `<polygon points="0,28 ${pts} 100,28" fill="${color}22" stroke="none" />
         <polyline points="${pts}" fill="none" stroke="${color}" stroke-width="1.5" />`
      : `<polyline points="${pts}" fill="none" stroke="${color}" stroke-width="1.5" />`;
  };
  return `<svg class="spark" viewBox="0 0 100 30" preserveAspectRatio="none">
    ${line(rxHistory, "#4fbf74", true)}${line(txHistory, "#5aa9e6", false)}</svg>`;
}

/** Fan card body: live RPM from unprivileged SMC reads plus a rolling
 * sparkline. Read-only by design — fan *control* is an SMC write and needs
 * the signed privileged helper, so no control buttons are offered. */
function fanBody(fans: FanStatus[]): string {
  if (fans.length === 0)
    return `<div class="value">—<small> RPM</small></div>
     <div class="foot" style="margin-top:14px">${t("st.fan.none")}</div>`;
  const rpm = Math.round(Math.max(...fans.map((f) => f.actual_rpm)));
  const maxRpm = fans[0].max_rpm;
  const minRpm = fans[0].min_rpm;
  // Scale against the hardware max when known, so an idling fan draws a low
  // line instead of autoscale pinning a flat idle curve to the top.
  const buf = maxRpm ? fanHistory.map((v) => Math.min(100, (v / maxRpm) * 100)) : fanHistory;
  const detail =
    fans.length > 1
      ? `${fans.map((f) => Math.round(f.actual_rpm)).join(" / ")} RPM`
      : minRpm !== null && maxRpm !== null
        ? `${t("st.fan.range")} ${Math.round(minRpm)}–${Math.round(maxRpm)}`
        : "";
  return `<div class="value">${rpm}<small> RPM</small></div>
   ${sparkline(buf, "var(--accent)", !maxRpm)}
   <div class="foot">${detail ? `${detail} · ` : ""}${t("st.fan.macos")}</div>`;
}

/** One metric card shell. */
function card(head: string, tag: string, body: string): string {
  return `<div class="card">
    <div class="card-head">${head}${tag ? `<span class="tag badge">${tag}</span>` : ""}</div>
    ${body}
  </div>`;
}

/** Skeleton card grid: the instant first paint before any snapshot exists,
 * so opening the tab never shows a blank screen while sampling runs. */
function renderSkeleton(container: HTMLElement): void {
  const card = `<div class="card">
    <div class="skel-line" style="width:40%"></div>
    <div class="skel-line" style="width:55%;height:22px;margin-top:14px"></div>
    <div class="skel-line" style="width:72%;margin-top:14px"></div>
  </div>`;
  container.innerHTML = `
    <div class="content-narrow" style="max-width:1280px">
      <div class="cards status-grid">${card.repeat(8)}</div>
      <div class="panel">
        <div class="skel-line" style="width:30%"></div>
        <div class="skel-line" style="width:90%;margin-top:12px"></div>
        <div class="skel-line" style="width:85%;margin-top:8px"></div>
      </div>
    </div>`;
}

/** Fetch one snapshot and re-render. */
async function refresh(container: HTMLElement): Promise<void> {
  let s: StatusSnapshot;
  try {
    s = await statusSnapshot();
  } catch (e) {
    // Only a first load with nothing on screen shows the error; a transient
    // IPC failure must not blank an already-rendered dashboard.
    if (!lastSnap) container.innerHTML = `<div class="placeholder">${esc(String(e))}</div>`;
    return;
  }
  lastSnap = s;
  push(gpuHistory, s.gpu.utilization_percent ?? 0);
  push(rxHistory, s.network.rx_rate_bps);
  push(txHistory, s.network.tx_rate_bps);
  // `?? []` degrades gracefully when the UI is hot-reloaded against an older
  // backend whose snapshot has no fans field yet.
  if ((s.fans ?? []).length > 0)
    push(fanHistory, Math.max(...s.fans.map((f) => f.actual_rpm)));
  renderSnapshot(container, s);
}

/** Render one snapshot (pure DOM, no fetching — also used for the instant
 * repaint from `lastSnap` when the tab is re-entered). */
function renderSnapshot(container: HTMLElement, s: StatusSnapshot): void {
  const fans = s.fans ?? [];
  const { score, message } = health(s);
  const memPct = (s.memory_used_bytes / s.memory_total_bytes) * 100;
  const root = s.disks.find((d) => d.mount_point === "/") ?? s.disks[0];
  const bootDate = new Date(Date.now() - s.uptime_seconds * 1000);
  const topProc = s.top_processes[0];

  const healthCard = `<div class="card health-card">
    <div class="card-head">☀ ${t("st.health")}
      <span class="tag">
        <span class="badge">${esc(s.hardware.chip.replace("Apple ", ""))}</span>
        <span class="badge">${s.hardware.memory_gb} GB</span>
        <span class="badge">macOS ${esc(s.hardware.os_version)}</span>
      </span>
    </div>
    <div>
      <div class="value">${score}<small> ${score >= 85 ? t("st.good") : t("st.fair")}</small></div>
      <div class="foot" style="margin-top:6px">${esc(message)}</div>
      <div class="foot" style="margin-top:14px">${t("st.uptime")} ${humanUptime(s.uptime_seconds)} · ${t("st.since")} ${bootDate.getMonth() + 1}/${bootDate.getDate()}</div>
    </div>
  </div>`;

  const cpuCard = card(
    `⌁ ${t("st.cpu")}`,
    `×${s.cpu_count}`,
    `<div class="value">${s.cpu_usage_percent.toFixed(0)}<small>%</small></div>
     ${coreBars(s.per_core_percent)}
     <div class="foot">${loadLabel(s)} · ${t("st.load")} ${s.load_avg_1m.toFixed(1)} / ${s.cpu_count} ${t("st.cores")}</div>`,
  );

  const gpuCard = card(
    `▣ ${t("st.gpu")}`,
    s.gpu.core_count ? `${s.gpu.core_count} ${t("st.gpucores")}` : "",
    `<div class="value">${s.gpu.utilization_percent ?? 0}<small>%</small></div>
     ${sparkline(gpuHistory, "var(--warn)")}
     <div class="foot">${loadLabel(s)} · ${s.gpu.core_count ?? "—"} ${t("st.gpucores")}</div>`,
  );

  const memCard = card(
    `▤ ${t("st.mem")}`,
    s.memory_pressure_percent !== null
      ? `${t("st.pressure")} ${s.memory_pressure_percent}%`
      : humanBytes(s.memory_total_bytes),
    `<div class="value">${memPct.toFixed(0)}<small>%</small></div>
     <div class="meter"><div style="width:${Math.min(100, memPct)}%"></div></div>
     <div class="foot">${humanBytes(s.memory_used_bytes)} · ${t("st.swap")} ${humanBytes(s.swap_used_bytes)}</div>`,
  );

  const batteryCard = s.battery
    ? card(
        `▮ ${t("st.battery")}`,
        s.battery.health_percent !== null
          ? `${t("st.health.badge")} ${s.battery.health_percent}%`
          : "",
        `<div class="value">${s.battery.percent}<small>% ${s.battery.charging ? "⚡" : ""} ${
          s.battery.watts !== null && s.battery.watts > 0.5
            ? `${s.battery.watts.toFixed(0)}W`
            : t("st.charged")
        }</small></div>
       <div class="foot">${s.battery.cycle_count ?? "—"} ${t("st.cycles")} · ${
         s.battery.temperature_c !== null ? formatTemp(s.battery.temperature_c) : "—"
       }</div>
       ${topProc ? `<div class="foot">🔥 ${t("st.topdrain")} ${esc(topProc.name)} · ${topProc.cpu_percent.toFixed(1)}</div>` : ""}`,
      )
    : "";

  const diskCard = root
    ? card(
        `◫ ${t("st.disk")}`,
        humanBytes(root.total_bytes),
        `<div class="value">${humanBytes(root.available_bytes)}<small> ${t("st.avail")}</small></div>
       <div class="meter disk-meter"><div style="width:${(((root.total_bytes - root.available_bytes) / root.total_bytes) * 100).toFixed(0)}%"></div></div>
       <div class="foot">${t("st.used")} ${humanBytes(root.total_bytes - root.available_bytes)} · ${(((root.total_bytes - root.available_bytes) / root.total_bytes) * 100).toFixed(0)}%</div>`,
      )
    : "";

  const netCard = card(
    `⇅ ${t("st.net")}`,
    s.network.interface ? esc(s.network.interface) : "",
    `<div class="value">${humanBytes(s.network.rx_rate_bps)}<small>/s</small></div>
     ${netChart()}
     <div class="foot">↑ ${humanBytes(s.network.tx_rate_bps)}/s${s.network.interface ? ` · ${esc(s.network.interface)}` : ""}</div>`,
  );

  const fanCard = card(`✽ ${t("st.fan")}`, fans.length > 1 ? `×${fans.length}` : "", fanBody(fans));

  container.innerHTML = `
    <div class="content-narrow" style="max-width:1280px">
      <div class="cards status-grid">
        ${healthCard}${cpuCard}${gpuCard}${memCard}
        ${batteryCard}${diskCard}${netCard}${fanCard}
      </div>
      <div class="panel" id="proc-panel">${procTable(s)}</div>
      <div class="muted" style="text-align:center">${esc(s.platform)} · ${esc(s.host)}</div>
    </div>`;

  wireProcTable(container);

}

/** Full process table markup (thead + tbody) from a snapshot. */
function procTable(s: StatusSnapshot): string {
  return `<table class="proc-table">
    <thead><tr>
      <th data-col="name" class="sortable">${t("st.name")} (${s.top_processes.length}) ${arrow("name")}</th>
      <th data-col="pid" class="num sortable">PID ${arrow("pid")}</th>
      <th data-col="cpu" class="num sortable">CPU ${arrow("cpu")}</th>
      <th data-col="energy" class="num sortable">${t("st.energy")} ${arrow("energy")}</th>
      <th data-col="mem" class="num sortable">${t("st.mem")} ${arrow("mem")}</th>
    </tr></thead>
    <tbody>${procRows(s.top_processes)}</tbody>
  </table>`;
}

/** Wire header sorting (instant, from the cached snapshot), row clicks, and
 * lazy icon fill. Called after every table (re)render. */
function wireProcTable(container: HTMLElement): void {
  const panel = container.querySelector<HTMLElement>("#proc-panel");
  if (!panel) return;

  panel.querySelectorAll<HTMLElement>("th.sortable").forEach((th) => {
    th.addEventListener("click", () => {
      const col = th.dataset.col as typeof sortCol;
      if (sortCol === col) sortDesc = !sortDesc;
      else {
        sortCol = col;
        sortDesc = col !== "name";
      }
      // Instant client-side resort — no snapshot refetch, no latency.
      if (lastSnap) {
        panel.innerHTML = procTable(lastSnap);
        wireProcTable(container);
      }
    });
  });

  panel.querySelectorAll<HTMLElement>("tr.proc-row").forEach((row) => {
    row.addEventListener("click", () => void openProcessModal(Number(row.dataset.pid)));
  });

  // Lazy icon fill for uncached app processes — sequential and single-flight
  // so overlapping renders never multiply concurrent backend fetches.
  const pending = Array.from(
    panel.querySelectorAll<HTMLElement>("[data-prociconpath]"),
  );
  const uniquePaths = [...new Set(pending.map((el) => el.dataset.prociconpath!))].filter(
    (path) => !procIcons.has(path),
  );
  if (iconLoopRunning || uniquePaths.length === 0) return;
  iconLoopRunning = true;
  void (async () => {
    try {
      for (const path of uniquePaths) {
        try {
          procIcons.set(path, await appIcon(path));
        } catch {
          procIcons.set(path, null);
        }
      }
    } finally {
      iconLoopRunning = false;
    }
    // Applied on the next 2s refresh; also patch in place for snappiness.
    pending.forEach((el) => {
      const icon = procIcons.get(el.dataset.prociconpath!);
      if (icon && el.isConnected) {
        const img = document.createElement("img");
        img.className = "proc-icon";
        img.src = icon;
        el.replaceWith(img);
      }
    });
  })();
}


/** The process detail modal (reference design image: OrbStack Helper). */
async function openProcessModal(pid: number): Promise<void> {
  const backdrop = document.createElement("div");
  backdrop.className = "confirm-backdrop";
  backdrop.innerHTML = `<div class="proc-sheet"><div class="placeholder">…</div></div>`;
  document.body.appendChild(backdrop);
  backdrop.addEventListener("click", (ev) => {
    if (ev.target === backdrop) backdrop.remove();
  });

  const d: ProcessDetail | null = await processDetail(pid).catch(() => null);
  const sheet = backdrop.querySelector<HTMLElement>(".proc-sheet")!;
  if (!d) {
    sheet.innerHTML = `<div class="placeholder">${t("proc.killfail")}</div>`;
    return;
  }

  const icon = d.app_path ? procIcons.get(d.app_path) : null;
  const chain = [...d.parent_chain, [d.pid, d.name] as [number, string]]
    .map(
      ([cpid, cname], i, arr) =>
        `<span class="${i === arr.length - 1 ? "" : "muted"}">${esc(cname)} <span class="muted">${cpid}</span></span>`,
    )
    .join(" › ");
  const row = (label: string, value: string) =>
    value
      ? `<div class="proc-kv"><span class="k muted">${label}</span><span class="v">${value}</span></div>`
      : "";

  sheet.innerHTML = `
    <div class="proc-head">
      ${icon ? `<img class="proc-icon big" src="${icon}" alt="" />` : `<span class="proc-icon big fallback">⚙︎</span>`}
      <h2>${esc(d.name)}</h2>
      <button class="chev" id="pm-close">✕</button>
    </div>
    <div class="muted" style="font-family:var(--font-mono);font-size:12px;margin-bottom:12px">
      PID ${d.pid} · CPU ${d.cpu_percent.toFixed(1)}% · MEM ${humanBytes(d.memory_bytes)}
      ${d.user ? ` · ${esc(d.user)}` : ""}${d.app_path ? ` · ${t("proc.from")} ${esc(d.app_path.split("/").pop()?.replace(".app", "") ?? "")}` : ""}
    </div>
    <div class="muted" style="font-family:var(--font-mono);font-size:12px;margin-bottom:14px">${chain}</div>
    ${row(t("proc.threads"), d.threads !== null ? String(d.threads) : "")}
    ${row(t("proc.openfiles"), d.open_files !== null ? String(d.open_files) : "")}
    ${row(t("proc.diskio"), `${humanBytes(d.disk_read_bytes)} R · ${humanBytes(d.disk_written_bytes)} W`)}
    ${row(
      t("proc.ports"),
      d.listen_ports.length
        ? `${d.listen_ports.map(esc).join(", ")} <span class="badge warn">${t("proc.ports.warn")}</span>`
        : "",
    )}
    ${row(t("proc.children"), String(d.children))}
    ${row(t("proc.user"), d.user ? esc(d.user) : "")}
    ${row(t("proc.runtime"), humanRuntime(d.run_time_seconds))}
    ${row(t("proc.cwd"), d.cwd ? esc(d.cwd) : "")}
    ${row(t("proc.exe"), d.exe ? esc(d.exe) : "")}
    ${d.cmd.length ? `<div class="proc-kv"><span class="k muted">${t("proc.cmd")}</span></div><div class="proc-cmd">${esc(d.cmd.join(" "))}</div>` : ""}
    <div class="confirm-actions">
      <button id="pm-copy">${t("proc.copy")}</button>
      ${d.exe ? `<button id="pm-reveal">${t("proc.reveal")}</button>` : ""}
      <button id="pm-term">${t("proc.term")}</button>
      <button id="pm-kill" class="danger">${t("proc.kill")}</button>
    </div>`;

  sheet.querySelector("#pm-close")!.addEventListener("click", () => backdrop.remove());
  sheet.querySelector("#pm-copy")!.addEventListener("click", async (ev) => {
    const summary = `${d.name} (pid ${d.pid}) cpu ${d.cpu_percent.toFixed(1)}% mem ${humanBytes(d.memory_bytes)}\n${d.exe ?? ""}\n${d.cmd.join(" ")}`;
    await navigator.clipboard.writeText(summary);
    (ev.target as HTMLElement).textContent = t("proc.copied");
  });
  sheet.querySelector("#pm-reveal")?.addEventListener("click", () => {
    if (d.exe) void revealInFinder(d.exe);
  });
  const kill = (force: boolean) => async (ev: Event) => {
    try {
      await signalProcess(d.pid, force);
      backdrop.remove();
    } catch {
      (ev.target as HTMLElement).textContent = t("proc.killfail");
    }
  };
  sheet.querySelector("#pm-term")!.addEventListener("click", kill(false));
  sheet.querySelector("#pm-kill")!.addEventListener("click", kill(true));
}

/** Sort indicator for a column header. */
function arrow(col: string): string {
  if (col !== sortCol) return "⇅";
  return sortDesc ? "▾" : "▴";
}

/** Sorted process rows with CPU bars and hot flames. */
function procRows(procs: ProcessInfo[]): string {
  const sorted = [...procs].sort((a, b) => {
    const cmp =
      sortCol === "name"
        ? a.name.localeCompare(b.name)
        : sortCol === "pid"
          ? a.pid - b.pid
          : sortCol === "cpu"
            ? a.cpu_percent - b.cpu_percent
            : sortCol === "energy"
              ? a.cpu_time_ms - b.cpu_time_ms
              : a.memory_bytes - b.memory_bytes;
    return sortDesc ? -cmp : cmp;
  });
  const maxCpu = Math.max(1, ...sorted.map((p) => p.cpu_percent));
  return sorted
    .map((p) => {
      const hot = p.cpu_percent > 100;
      const width = Math.max(2, Math.round((p.cpu_percent / maxCpu) * 60));
      const cachedIcon = p.app_path ? procIcons.get(p.app_path) : null;
      const iconHtml = cachedIcon
        ? `<img class="proc-icon" src="${cachedIcon}" alt="" />`
        : `<span class="proc-icon fallback" ${p.app_path ? `data-prociconpath="${esc(p.app_path)}"` : ""}>⚙︎</span>`;
      return `<tr class="proc-row" data-pid="${p.pid}">
        <td><span class="proc-dot ${hot ? "hot" : ""}"></span>${iconHtml} ${esc(p.name)} ${hot ? "🔥" : ""}</td>
        <td class="muted num">${p.pid}</td>
        <td class="num"><span class="cpu-bar ${hot ? "hot" : ""}" style="width:${width}px"></span>
          <span style="color:${hot ? "var(--danger)" : "inherit"}">${p.cpu_percent.toFixed(1)}</span></td>
        <td class="muted num">${humanCpuTime(p.cpu_time_ms)}</td>
        <td class="num">${humanBytes(p.memory_bytes)}</td>
      </tr>`;
    })
    .join("");
}

/** Re-render immediately when the window becomes visible again. */
let onVisible: (() => void) | null = null;

export const dashboard: View = {
  mount(container) {
    // Instant first paint: the last snapshot when returning to the tab, a
    // skeleton grid on the very first open — never a blank screen while the
    // snapshot samples.
    if (lastSnap) renderSnapshot(container, lastSnap);
    else renderSkeleton(container);
    void refresh(container);
    // Skip refreshes while the window is hidden (close-to-tray keeps the view
    // mounted): polling an invisible dashboard is pure CPU waste.
    timer = window.setInterval(() => {
      if (!document.hidden) void refresh(container);
    }, 2000);
    onVisible = () => {
      if (!document.hidden) void refresh(container);
    };
    document.addEventListener("visibilitychange", onVisible);
  },
  unmount() {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
    if (onVisible !== null) {
      document.removeEventListener("visibilitychange", onVisible);
      onVisible = null;
    }
  },
};
