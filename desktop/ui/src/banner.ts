// 报头下方的通栏提示条：目前只有"有新版本"一种。

import { esc } from "./format";
import { updateInstall } from "./ipc";
import { t } from "./i18n";
import type { UpdateInfo } from "./types";

/** 提示条挂载点；index.html 里在 nav 与 main 之间。 */
function slot(): HTMLElement | null {
  return document.getElementById("banner");
}

/** 收起提示条。 */
function dismiss(): void {
  const el = slot();
  if (el) el.innerHTML = "";
}

/**
 * 渲染一条提示。`actions` 是右侧按钮的 HTML，`closable` 决定有没有 ✕。
 * 强制更新时 closable 为 false —— 那是出事版本的止血通道，不给关。
 */
function render(text: string, actions: string, closable: boolean): void {
  const el = slot();
  if (!el) return;
  el.innerHTML = `
    <div class="banner">
      <span class="banner-text">${text}</span>
      <span class="banner-rule"></span>
      <span class="banner-actions">${actions}</span>
      ${closable ? `<button class="banner-close" id="banner-x">✕</button>` : ""}
    </div>`;
  el.querySelector("#banner-x")?.addEventListener("click", dismiss);
}

/** 有新版本可用。`mandatory` 为真时不给关，也不给"稍后"。 */
export function showUpdateBanner(info: UpdateInfo): void {
  const text = info.mandatory
    ? t("upd.self.mandatory", { version: esc(info.version) })
    : t("upd.self.available", { version: esc(info.version) });
  render(
    text,
    `${info.mandatory ? "" : `<button id="upd-later">${t("upd.self.later")}</button>`}
     <button id="upd-go" class="primary">${t("upd.self.go")}</button>`,
    !info.mandatory,
  );
  document.getElementById("upd-later")?.addEventListener("click", dismiss);
  document.getElementById("upd-go")?.addEventListener("click", () => {
    void install(info);
  });
}

/** 执行安装：按钮变进度，成功后应用自行重启，所以没有"成功"分支。 */
async function install(_info: UpdateInfo): Promise<void> {
  const button = document.getElementById("upd-go") as HTMLButtonElement | null;
  if (button) {
    button.disabled = true;
    button.textContent = t("upd.self.downloading");
  }
  try {
    await updateInstall((progress) => {
      if (!button) return;
      const percent = progress.total
        ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
        : null;
      button.textContent =
        percent === null ? t("upd.self.downloading") : `${percent}%`;
    });
  } catch {
    if (button) {
      button.disabled = false;
      button.textContent = t("upd.self.retry");
    }
  }
}
