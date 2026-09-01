// 报头下方的通栏提示条：首次遥测告知与"有新版本"。
//
// 两件事共用一个槽位，因为它们绝不该同时出现在屏幕上——首次启动的用户不会
// 有更新可装，而装了很久的用户早就看过遥测告知了。共用一个槽位也省掉了
// 布局互相挤压的问题。

import { esc } from "./format";
import { setTelemetryEnabled, telemetryNoticeAck, updateInstall } from "./ipc";
import { t } from "./i18n";
import { track } from "./telemetry";
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

/** 首次启动的遥测告知：默认已开，给一个当场关掉的入口。 */
export function showTelemetryNotice(): void {
  render(
    t("tele.notice"),
    `<a href="#/settings" id="tele-more">${t("tele.notice.detail")}</a>
     <button id="tele-off">${t("tele.notice.off")}</button>
     <button id="tele-ok" class="primary">${t("tele.notice.ok")}</button>`,
    false,
  );
  // 无论用户点哪个都算"已告知"：横幅的职责是让他知道，而不是逼他表态。
  const ack = () => {
    void telemetryNoticeAck();
    dismiss();
  };
  document.getElementById("tele-ok")?.addEventListener("click", ack);
  document.getElementById("tele-more")?.addEventListener("click", ack);
  document.getElementById("tele-off")?.addEventListener("click", () => {
    void setTelemetryEnabled(false);
    ack();
  });
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
async function install(info: UpdateInfo): Promise<void> {
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
    // error_occurred 已由 ipc 包装层记过，这里只补一条带版本号的失败事件。
    track({
      kind: "self_update",
      from: info.current_version,
      to: info.version,
      result: "failed",
    });
    if (button) {
      button.disabled = false;
      button.textContent = t("upd.self.retry");
    }
  }
}
