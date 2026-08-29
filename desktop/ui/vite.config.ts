// Vite config for the Tauri frontend: fixed port so tauri.conf.json's devUrl
// stays stable, and no auto-open (Tauri owns the window).
import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    target: "safari15",
  },
});
