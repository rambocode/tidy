// Settings (reference design): a centered sheet with 通用/工具/菜单栏 tabs.
// Rows are title + description left, control right. Only real capabilities
// appear — native-only features (wipe-screen guard, caffeinate, license)
// are not faked.

import {
  appMeta,
  appSettings,
  autostartGet,
  autostartSet,
  fdaStatus,
  fdaHelperShow,
  fdaHelperHide,
  openFdaSettings,
  purgePathsGet,
  setUpdateAutocheck,
  touchidStatus,
  updateCheck,
  whitelistGet,
  whitelistSet,
} from "../ipc";
import { showUpdateBanner } from "../banner";
import type { AppSettings } from "../types";
import { PRODUCT_NAME } from "../brand";
import { esc } from "../format";
import { lang, langMode, setLang, t } from "../i18n";
import { listen } from "@tauri-apps/api/event";
import { applyBackendPrefs, dockerIdleMonths, pref, purgeIdleDays, setPref } from "../prefs";
import type { View } from "../router";

type Tab = "general" | "tools" | "menubar";
let tab: Tab = "general";

/** One settings row: title/desc left, control HTML right. */
function row(title: string, desc: string, control: string): string {
  return `<div class="set-row">
    <div class="set-text">
      <div class="set-title">${title}</div>
      <div class="set-desc muted">${desc}</div>
    </div>
    <div class="set-control">${control}</div>
  </div>`;
}

/** Segmented control HTML. */
function segmented(id: string, options: [string, string][], active: string): string {
  return `<div class="segmented" id="${id}">${options
    .map(
      ([value, label]) =>
        `<button data-value="${value}" class="${value === active ? "on" : ""}">${label}</button>`,
    )
    .join("")}</div>`;
}

/** Small numeric input (days / months thresholds). */
function numberInput(id: string, value: number, unit: string): string {
  return `<label class="num-field"><input type="number" id="${id}" min="1" max="3650" value="${value}" /> ${unit}</label>`;
}

/** Toggle switch HTML. */
function toggle(id: string, on: boolean): string {
  return `<button class="toggle ${on ? "on" : ""}" id="${id}"></button>`;
}

export const settings: View = {
  unmount() {
    stopFdaPoll();
    unlistenHelper?.();
    unlistenHelper = undefined;
  },
  async mount(container) {
    const [meta, fda, autostart, wl, purgePaths, touchid, backend] = await Promise.all([
      appMeta(),
      fdaStatus(),
      autostartGet(),
      whitelistGet(),
      purgePathsGet(),
      touchidStatus(),
      appSettings(),
    ]);

    const draw = () => {
      const tabs: [Tab, string][] = [
        ["general", t("set.tab.general")],
        ["tools", t("set.tab.tools")],
        ["menubar", t("set.tab.menubar")],
      ];
      const body =
        tab === "general"
          ? generalTab(fda, autostart, backend)
          : tab === "tools"
            ? toolsTab(wl.patterns, purgePaths, touchid.enabled)
            : menubarTab();

      container.innerHTML = `
        <div class="settings-sheet">
          <div class="preview-head" style="margin-bottom:8px">
            <h1 style="font-size:22px;margin:0">${t("set.title")}</h1>
            <button class="chev" id="set-close">✕</button>
          </div>
          <div class="mast-tabs" style="margin-bottom:18px">
            ${tabs
              .map(
                ([id, label]) =>
                  `<a href="#" data-tab="${id}" class="${id === tab ? "active" : ""}">${label}</a>`,
              )
              .join("")}
          </div>
          ${body}
          <div class="muted" style="margin-top:16px;font-family:var(--font-mono);font-size:11px">
            ${PRODUCT_NAME} v${esc(meta.app_version)} · ${esc(meta.protection_data_sha256.slice(0, 12))}…
            · ${meta.helper_available ? t("set.helper.on") : t("set.helper.off")}
          </div>
        </div>`;

      container.querySelector("#set-close")!.addEventListener("click", () => {
        history.length > 1 ? history.back() : (location.hash = "#/clean");
      });
      container.querySelectorAll<HTMLAnchorElement>("a[data-tab]").forEach((a) => {
        a.addEventListener("click", (ev) => {
          ev.preventDefault();
          tab = a.dataset.tab as Tab;
          draw();
        });
      });
      wire(container, draw);
    };
    draw();
  },
};

/** 通用 tab. */
function generalTab(fda: boolean, autostart: boolean, backend: AppSettings): string {
  const mode = langMode();
  return [
    row(
      t("set.fda"),
      t("set.fda.desc"),
      fda
        ? `<span class="badge ok">${t("set.fda.granted")}</span>`
        : `<button id="fda-open">${t("set.fda.open")}</button>`,
    ),
    row(
      t("set.lang"),
      t("set.lang.desc", { cur: lang() === "zh" ? "中文" : "English" }),
      segmented("seg-lang", [["auto", t("set.lang.auto")], ["zh", "中文"], ["en", "EN"]], mode),
    ),
    row(
      t("set.temp"),
      t("set.temp.desc"),
      segmented("seg-temp", [["c", "°C"], ["f", "°F"]], pref("temp-unit", "c")),
    ),
    row(t("set.autostart"), t("set.autostart.desc"), toggle("tg-autostart", autostart)),
    row(
      t("set.selfupdate"),
      t("set.selfupdate.desc"),
      `<div class="set-inline">
         <button id="upd-check">${t("set.selfupdate.check")}</button>
         ${toggle("tg-autocheck", backend.update_autocheck)}
       </div>`,
    ),
    `<div id="upd-check-status" class="muted set-note"></div>`,
  ].join("");
}

/** 工具 tab. */
function toolsTab(patterns: string[], purgePaths: string[] | null, touchidOn: boolean): string {
  return [
    row(
      t("set.delmode"),
      t("set.delmode.desc"),
      segmented(
        "seg-delmode",
        [
          ["permanent", t("set.delmode.perm")],
          ["trash", t("set.delmode.trash")],
        ],
        pref("delete-mode", "permanent"),
      ),
    ),
    `<div class="set-row" style="flex-direction:column;align-items:stretch;gap:8px">
      <div class="set-text">
        <div class="set-title">${t("set.protect")}</div>
        <div class="set-desc muted">${t("set.protect.desc")}</div>
      </div>
      <textarea id="wl">${esc(patterns.join("\n"))}</textarea>
      <div style="display:flex;gap:8px;align-items:center">
        <button id="wl-save">${t("set.save")}</button>
        <span id="wl-status" class="muted"></span>
      </div>
      <div id="wl-warnings"></div>
    </div>`,
    row(
      t("set.purgepaths"),
      purgePaths ? purgePaths.map(esc).join("<br>") : t("set.purgedefault"),
      "",
    ),
    row(
      t("set.purgeidle"),
      t("set.purgeidle.desc"),
      numberInput("num-purgeidle", purgeIdleDays(), t("set.unit.days")),
    ),
    row(
      t("set.dockeridle"),
      t("set.dockeridle.desc"),
      numberInput("num-dockeridle", dockerIdleMonths(), t("set.unit.months")),
    ),
    row(
      t("set.touchid"),
      t("set.touchid.hint"),
      touchidOn
        ? `<span class="badge ok">${t("set.enabled")}</span>`
        : `<span class="badge">${t("set.disabled")}</span>`,
    ),
  ].join("");
}

/** 菜单栏 tab. */
function menubarTab(): string {
  return [
    row(t("set.tray"), t("set.tray.desc"), toggle("tg-tray", pref("tray-visible", "1") === "1")),
    row(
      t("set.keeptray"),
      t("set.keeptray.desc"),
      toggle("tg-keeptray", pref("keep-in-tray", "0") === "1"),
    ),
  ].join("");
}

/** Poll timer started by the FDA button; cleared on grant and on unmount. */
let fdaPoll: number | undefined;
function stopFdaPoll(): void {
  if (fdaPoll !== undefined) window.clearInterval(fdaPoll);
  fdaPoll = undefined;
}

/** Backend "helper window closed" subscription; the page re-reads the
 * permission once so the row reflects a grant made while it was open. */
let unlistenHelper: (() => void) | undefined;

/** Wire the active tab's controls. */
function wire(container: HTMLElement, redraw: () => void): void {
  // "去授权": open the pane, float the drag helper beside it, and poll until
  // the toggle flips so the row turns into the "granted" badge by itself.
  unlistenHelper?.();
  void listen("fda-helper-closed", () => {
    stopFdaPoll();
    redraw();
  }).then((un) => {
    unlistenHelper = un;
  });
  container.querySelector("#fda-open")?.addEventListener("click", () => {
    void openFdaSettings();
    void fdaHelperShow();
    stopFdaPoll();
    fdaPoll = window.setInterval(() => {
      void fdaStatus().then((ok) => {
        if (!ok) return;
        stopFdaPoll();
        void fdaHelperHide();
        redraw();
      });
    }, 1500);
  });


  container.querySelectorAll<HTMLElement>(".segmented").forEach((seg) => {
    seg.querySelectorAll<HTMLButtonElement>("button").forEach((btn) => {
      btn.addEventListener("click", () => {
        const value = btn.dataset.value!;
        if (seg.id === "seg-lang") {
          setLang(value as "auto" | "zh" | "en");
          return; // setLang re-renders the route
        }
        if (seg.id === "seg-temp") setPref("temp-unit", value);
        if (seg.id === "seg-delmode") setPref("delete-mode", value);
        redraw();
      });
    });
  });

  const wireToggle = (id: string, onChange: (on: boolean) => void) => {
    const el = container.querySelector<HTMLButtonElement>(`#${id}`);
    el?.addEventListener("click", () => {
      const on = !el.classList.contains("on");
      el.classList.toggle("on", on);
      onChange(on);
    });
  };
  wireToggle("tg-autostart", (on) => void autostartSet(on));
  wireToggle("tg-autocheck", (on) => void setUpdateAutocheck(on));

  // 手动检查绕开 24 小时频率限制（auto=false）：用户主动点了就该真发请求。
  container.querySelector("#upd-check")?.addEventListener("click", async () => {
    const status = container.querySelector<HTMLElement>("#upd-check-status")!;
    status.textContent = t("set.selfupdate.checking");
    try {
      const info = await updateCheck(false);
      if (info) {
        status.textContent = t("set.selfupdate.found", { version: info.version });
        showUpdateBanner(info);
      } else {
        status.textContent = t("set.selfupdate.latest");
      }
    } catch {
      status.textContent = t("set.selfupdate.failed");
    }
  });
  wireToggle("tg-tray", (on) => {
    setPref("tray-visible", on ? "1" : "0");
    applyBackendPrefs();
  });
  wireToggle("tg-keeptray", (on) => {
    setPref("keep-in-tray", on ? "1" : "0");
    applyBackendPrefs();
  });

  // Threshold inputs save on every change; invalid values fall back to the
  // default at read time (see prefs.intPref).
  const wireNumber = (id: string, key: "purge-idle-days" | "docker-idle-months") => {
    container.querySelector<HTMLInputElement>(`#${id}`)?.addEventListener("change", (ev) => {
      setPref(key, (ev.target as HTMLInputElement).value.trim());
    });
  };
  wireNumber("num-purgeidle", "purge-idle-days");
  wireNumber("num-dockeridle", "docker-idle-months");

  container.querySelector("#wl-save")?.addEventListener("click", async () => {
    const textarea = container.querySelector<HTMLTextAreaElement>("#wl")!;
    const status = container.querySelector<HTMLElement>("#wl-status")!;
    const patterns = textarea.value
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l.length > 0);
    try {
      const warnings = await whitelistSet(patterns);
      status.textContent = t("set.saved");
      container.querySelector<HTMLElement>("#wl-warnings")!.innerHTML = warnings
        .map((w) => `<div class="badge warn" style="margin-top:6px">${esc(w)}</div>`)
        .join("");
    } catch (e) {
      status.textContent = String(e);
    }
  });
}
