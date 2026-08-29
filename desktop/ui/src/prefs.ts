// Local preferences (persisted in localStorage) + boot-time sync of the
// backend-affecting ones (tray visibility, close-to-tray).

import { setKeepInTray, traySetVisible } from "./ipc";

type Keys = "temp-unit" | "delete-mode" | "tray-visible" | "keep-in-tray";

/** Read a preference with a default. */
export function pref(key: Keys, def: string): string {
  return localStorage.getItem(`mole-${key}`) ?? def;
}

/** Write a preference. */
export function setPref(key: Keys, value: string): void {
  localStorage.setItem(`mole-${key}`, value);
}

/** Battery/thermal display unit. */
export const tempUnit = () => pref("temp-unit", "c");

/** Format a °C reading in the chosen unit. */
export function formatTemp(celsius: number): string {
  if (tempUnit() === "f") return `${((celsius * 9) / 5 + 32).toFixed(0)}°F`;
  return `${celsius.toFixed(0)}°C`;
}

/** Clean cache delete mode: "permanent" (default, CLI parity) or "trash". */
export const cleanTrashMode = () => pref("delete-mode", "permanent") === "trash";

/** Push the backend-affecting prefs at boot. */
export function applyBackendPrefs(): void {
  void traySetVisible(pref("tray-visible", "1") === "1");
  void setKeepInTray(pref("keep-in-tray", "0") === "1");
}
