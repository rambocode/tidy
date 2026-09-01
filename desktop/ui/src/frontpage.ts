// The front page shared by the three idle screens (清理 / 优化 / 分析).
//
// Every section opens the same way in the reference: a kicker line with a
// strapline on the far right, then a two-column spread — an outline numeral
// bleeding off the top edge, a 70px display headline with one word in rust, a
// ruled byline, the double-framed call to action with its running-time note,
// and a "编者按" rail carrying a drop-capped standfirst over a three-row
// ledger. Only the copy and the numbers differ per section, so the shell
// lives here once.

import { esc } from "./format";
import { t } from "./i18n";

/** One row of the rail's ledger; the last row is printed in rust. */
export interface FrontStat {
  label: string;
  value: string;
}

export interface FrontPageOptions {
  /** Rust mono line at the top left, e.g. "第 47 期 · 智能清理专栏". */
  kicker: string;
  /** Muted mono line at the top right. */
  strapline: string;
  /** Outline numeral bleeding off the top edge; "" prints nothing. */
  watermark: string;
  /** The analyze page's numeral is wider, so its geometry is smaller. */
  watermarkWide?: boolean;
  /** Display headline. Wrap the accented word in <em> — it turns rust. */
  headlineHtml: string;
  /** Byline desk, e.g. "本刊编辑部 · 磁盘管理组". */
  desk: string;
  /** Monospaced dateline beside the byline. */
  dateline: string;
  /** Optional block between the byline and the action (the scope row). */
  extraHtml?: string;
  /** Action verb; the arrow is added here. */
  action: string;
  /** Short mono note printed beside the action. */
  actionNote: string;
  /** Makes that note clickable (analyze uses it to reopen a cached result). */
  onActionNote?: () => void;
  /** Standfirst prose. Its first character is lifted into the drop cap. */
  noteBody: string;
  /** Three ledger rows under the standfirst. */
  stats: FrontStat[];
}

/**
 * Render the front page into `container` and return its action button so the
 * caller can wire the scan/check/analyse it starts.
 */
export function renderFrontPage(
  container: HTMLElement,
  opts: FrontPageOptions,
): HTMLButtonElement {
  // The drop cap is the standfirst's first character set at 54px; the rest of
  // the paragraph flows around it, exactly like the reference.
  const [dropCap, ...rest] = [...opts.noteBody];

  container.innerHTML = `
    <div class="front">
      <div class="front-head">
        <span class="kicker">${esc(opts.kicker)}</span>
        <span class="strapline">${esc(opts.strapline)}</span>
      </div>
      <div class="front-body">
        ${
          opts.watermark
            ? `<div class="front-watermark ${opts.watermarkWide ? "wide" : ""}" aria-hidden="true">${esc(
                opts.watermark,
              )}</div>`
            : ""
        }
        <div class="front-lead">
          <h1 class="front-headline">${opts.headlineHtml}</h1>
          <div class="byline">
            <span class="byline-rule"></span>
            <span class="byline-desk">${esc(opts.desk)}</span>
            <span class="byline-date">${esc(opts.dateline)}</span>
          </div>
          ${opts.extraHtml ?? ""}
          <div class="front-action">
            <button class="frame-cta double" id="front-action">${esc(opts.action)} →</button>
            ${
              opts.onActionNote
                ? `<button class="front-action-note link" id="front-action-note">${esc(opts.actionNote)}</button>`
                : `<span class="front-action-note">${esc(opts.actionNote)}</span>`
            }
          </div>
        </div>
        <aside class="front-note">
          <span class="front-note-title">${t("front.note")}</span>
          <p class="standfirst"><span class="dropcap">${esc(dropCap ?? "")}</span>${esc(rest.join(""))}</p>
          <div class="front-ledger">
            ${opts.stats
              .map(
                (stat, idx) => `<div class="led">
                  <span class="front-stat-label">${esc(stat.label)}</span>
                  <span class="dots"></span>
                  <span class="amt ${idx === opts.stats.length - 1 ? "lead" : ""}">${esc(stat.value)}</span>
                </div>`,
              )
              .join("")}
          </div>
        </aside>
      </div>
    </div>`;

  container
    .querySelector<HTMLButtonElement>("#front-action-note")
    ?.addEventListener("click", () => opts.onActionNote?.());

  return container.querySelector<HTMLButtonElement>("#front-action")!;
}
