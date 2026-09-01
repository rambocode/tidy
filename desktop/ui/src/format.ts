// Shared formatting helpers for the dense table views.

/** Human byte size, decimal units to match the CLI's bytes_to_human. */
export function humanBytes(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(2)}GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)}MB`;
  if (bytes >= 1_000) return `${Math.round(bytes / 1_000)}KB`;
  return `${bytes}B`;
}

/** Human size from KB (the unit the plan API uses). */
export function humanKb(kb: number): string {
  return humanBytes(kb * 1024);
}

/** Uptime seconds → "3d 4h" style. */
export function humanUptime(seconds: number): string {
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

/** Escape text for HTML interpolation, safe in element AND attribute context
 * (quotes included — log paths are untrusted bytes). */
export function esc(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

/** Split a formatted size like "135.02GB" into its numeral and unit so the
 * newspaper figure can set them at different sizes. A trailing ".00" is
 * dropped — a headline numeral does not print empty decimals. */
export function splitUnit(text: string): [string, string] {
  const m = /^([\d.]+)\s*(.*)$/.exec(text);
  if (!m) return [text, ""];
  return [m[1].replace(/\.0+$/, ""), m[2]];
}

/** Size step for a headline numeral. The 104px default is set for readings
 * like "23.4"; a six- or seven-digit total would run past its column, so long
 * numerals step down instead of overflowing. */
export function figureClass(numeral: string): string {
  if (numeral.length >= 7) return "xlong";
  if (numeral.length >= 5) return "long";
  return "";
}
