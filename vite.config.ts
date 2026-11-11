import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

const host = process.env.TAURI_DEV_HOST;
const page = (name: string) => fileURLToPath(new URL(`./${name}.html`, import.meta.url));

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    // WKWebView on macOS 11+, WebView2 and WebKitGTK all handle this.
    target: ["safari15", "chrome105"],
    rollupOptions: {
      input: {
        wallpaper: page("wallpaper"),
        viewer: page("viewer"),
        settings: page("settings"),
      },
    },
  },
});
