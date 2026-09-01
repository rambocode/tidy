// Hash router with a mount/unmount lifecycle. Renders the newspaper masthead
// (wordmark · section tabs · marginalia) above the double rule, applies the
// per-route body theme, and re-renders on language switches (setLang fires a
// hashchange). Settings and language switching live in the tray menu / the
// settings view, not in the masthead.

import { lang, t } from "./i18n";

export interface View {
  /** Render into the container; called on every navigation to this route. */
  mount(container: HTMLElement): void;
  /** Tear down timers/subscriptions; called before leaving the route. */
  unmount?(): void;
}

interface Route {
  hash: string;
  labelKey: string;
  theme: string;
  minor: boolean;
  hidden: boolean;
  view: View;
}

const routes: Route[] = [];
let current: View | null = null;

/** True while a view owns the masthead marginalia, so the clock stays quiet. */
let metaOverridden = false;
let clockTimer: number | null = null;

/** Register a view under a hash route; hidden routes stay navigable (tray) but render no nav link. */
export function register(
  hash: string,
  labelKey: string,
  theme: string,
  view: View,
  minor = false,
  hidden = false,
): void {
  routes.push({ hash, labelKey, theme, view, minor, hidden });
}

/** The dateline printed at the right of the masthead, e.g. "周二 · 9 月 1 日 · 20:56". */
function dateline(): string {
  const now = new Date();
  const locale = lang() === "zh" ? "zh-CN" : "en-US";
  const weekday = new Intl.DateTimeFormat(locale, { weekday: "short" }).format(now);
  const date = new Intl.DateTimeFormat(locale, { month: "long", day: "numeric" }).format(now);
  const time = new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(now);
  return `${weekday} · ${date} · ${time}`;
}

/**
 * Replace the masthead's right-hand marginalia with view-supplied copy
 * (already-escaped HTML). Passing null hands the slot back to the dateline.
 * Reset on every navigation, so a view must re-assert it after mounting.
 */
export function setNavMeta(html: string | null): void {
  const el = document.getElementById("nav-meta");
  metaOverridden = html !== null;
  if (!el) return;
  el.innerHTML = html ?? dateline();
}

/** Render the masthead for the active route. */
function renderNav(activeHash: string): void {
  const nav = document.getElementById("nav")!;
  const visible = routes.filter((r) => !r.hidden);
  const major = visible.filter((r) => !r.minor);
  const minor = visible.filter((r) => r.minor);
  const link = (r: Route, cls: string) =>
    `<a href="${r.hash}" class="${cls} ${r.hash === activeHash ? "active" : ""}">${t(r.labelKey)}</a>`;
  nav.innerHTML = `
    <div class="masthead">
      <span class="mast-wordmark">TIDY</span>
      <div class="mast-tabs">
        ${major.map((r) => link(r, "")).join("")}
        ${minor.length > 0 ? `<span class="mast-sep"></span>${minor.map((r) => link(r, "minor")).join("")}` : ""}
      </div>
      <span class="mast-meta" id="nav-meta">${dateline()}</span>
    </div>
    <div class="mast-rules"></div>`;
}

/** Navigate to the current location.hash (first route on unknown). */
function navigate(): void {
  const container = document.getElementById("view")!;
  const hash = location.hash || routes[0].hash;
  const route = routes.find((r) => r.hash === hash) ?? routes[0];
  current?.unmount?.();
  container.innerHTML = "";
  document.body.dataset.theme = route.theme;
  current = route.view;
  metaOverridden = false;
  renderNav(route.hash);
  route.view.mount(container);
}

/** Start routing. */
export function start(): void {
  window.addEventListener("hashchange", navigate);
  navigate();
  // The dateline is part of the page furniture, so it has to stay honest;
  // a view that claimed the slot keeps it until the next navigation.
  clockTimer = window.setInterval(() => {
    if (metaOverridden || document.hidden) return;
    const el = document.getElementById("nav-meta");
    if (el) el.textContent = dateline();
  }, 20_000);
}

/** Stop the masthead clock (tests / teardown). */
export function stop(): void {
  if (clockTimer !== null) {
    clearInterval(clockTimer);
    clockTimer = null;
  }
}
