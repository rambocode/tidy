// Floating helper window shown next to System Settings while the user grants
// Full Disk Access: a draggable app icon (drag it onto the FDA list), a
// looping "drag me" animation, and a poll that closes the window once the
// permission is detected.

import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { esc } from "./format";
import { t } from "./i18n";
import { fdaDragSource, fdaHelperHide, fdaStatus } from "./ipc";

/** Poll interval for "did the toggle flip yet". */
const POLL_MS = 1500;

/** Render the helper into the whole document and start the poll. */
export function mountFdaHelper(): void {
  document.body.dataset.theme = "";
  document.body.classList.add("fda-helper-body");
  const app = document.getElementById("app")!;
  app.innerHTML = `
    <div class="fda-helper" data-tauri-drag-region>
      <div class="fda-stage" id="fda-icon">
        <img src="/tidy-app-icon.png" alt="Tidy" class="fda-app-icon" draggable="false" />
        <img src="/tidy-app-icon.png" alt="" class="fda-app-ghost" draggable="false" />
        <div class="fda-list">
          <span></span><span></span><span class="target"></span>
        </div>
      </div>
      <div class="fda-text" data-tauri-drag-region>
        <div class="fda-title">${esc(t("fda.drag.title"))}</div>
        <div class="fda-sub muted">${esc(t("fda.drag.sub"))}</div>
      </div>
      <button class="chev" id="fda-close" title="${esc(t("flow.cancel"))}">✕</button>
    </div>`;

  // Native file drag: mousedown on the icon starts an OS drag carrying the
  // .app path, so dropping on the FDA list adds Tidy exactly like Finder.
  app.querySelector<HTMLElement>("#fda-icon")!.addEventListener("mousedown", (ev) => {
    if (ev.button !== 0) return;
    ev.preventDefault();
    void fdaDragSource()
      .then((src) =>
        startDrag({ item: [src.app_path], icon: src.icon_path }, (ev) => {
          // Job done once the drop landed: the row is in the list, the user
          // only has to flip the toggle. A cancelled drag keeps the helper.
          if (ev.result === "Dropped") void fdaHelperHide();
        }),
      )
      .catch(() => {
        /* drag unsupported here (e.g. no bundle): the text still explains the + route */
      });
  });
  app.querySelector("#fda-close")!.addEventListener("click", () => void fdaHelperHide());

  // Close ourselves as soon as the permission lands.
  const timer = window.setInterval(() => {
    void fdaStatus().then((ok) => {
      if (ok) {
        window.clearInterval(timer);
        void fdaHelperHide();
      }
    });
  }, POLL_MS);
}
