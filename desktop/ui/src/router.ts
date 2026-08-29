// Hash router with a mount/unmount lifecycle. Renders the centered pill
// navigation, applies the per-route body theme, and re-renders on language
// switches (setLang fires a hashchange). Settings and language switching
// live in the tray menu / settings view, not in the pill nav.

import { PRODUCT_LOGO_ALT, PRODUCT_LOGO_SRC } from "./brand";
import { t } from "./i18n";

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

/** Render the pill nav for the active route. */
function renderNav(activeHash: string): void {
  const nav = document.getElementById("nav")!;
  const visible = routes.filter((r) => !r.hidden);
  const major = visible.filter((r) => !r.minor);
  const minor = visible.filter((r) => r.minor);
  nav.innerHTML = `
    <div class="pill">
      <div class="logo"><img src="${PRODUCT_LOGO_SRC}" alt="${PRODUCT_LOGO_ALT}" /></div>
      ${major
        .map(
          (r) =>
            `<a href="${r.hash}" class="${r.hash === activeHash ? "active" : ""}">${t(r.labelKey)}</a>`,
        )
        .join("")}
      ${
        minor.length > 0
          ? `<div class="sep"></div>${minor
              .map(
                (r) =>
                  `<a href="${r.hash}" class="minor ${r.hash === activeHash ? "active" : ""}">${t(r.labelKey)}</a>`,
              )
              .join("")}`
          : ""
      }
    </div>`;
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
  renderNav(route.hash);
  route.view.mount(container);
}

/** Start routing. */
export function start(): void {
  window.addEventListener("hashchange", navigate);
  navigate();
}
