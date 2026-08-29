// App boot: register themed views, start the pill-nav router, sync
// backend-affecting prefs, and listen for tray-menu navigation events.

import { listen } from "@tauri-apps/api/event";
import { applyBackendPrefs } from "./prefs";
import { register, start } from "./router";
import { analyze } from "./views/analyze";
import { clean } from "./views/clean";
import { dashboard } from "./views/dashboard";
import { optimize } from "./views/optimize";
import { settings } from "./views/settings";
import { uninstall } from "./views/uninstall";

// Purge and installer are folded into the clean flow; history is log-only
// storage (no screen).
register("#/clean", "nav.clean", "clean", clean);
register("#/apps", "nav.apps", "apps", uninstall);
register("#/optimize", "nav.optimize", "optimize", optimize);
register("#/analyze", "nav.analyze", "analyze", analyze);
register("#/status", "nav.status", "status", dashboard);
// Settings is tray-only: navigable via the tray menu, no pill-nav link.
register("#/settings", "nav.settings", "", settings, true, true);

start();
applyBackendPrefs();
void listen<string>("mole-nav", (event) => {
  location.hash = event.payload;
});
