import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Colors, DeepPartial, LibraryView, Settings, Snapshot } from "./types";

const isWindows = navigator.userAgent.includes("Windows");
const BASE = isWindows ? "http://galeria.localhost/" : "galeria://localhost/";

export const imageUrl = {
  original: (id: string) => `${BASE}original/${encodeURIComponent(id)}`,
  thumb: (id: string) => `${BASE}thumb/${encodeURIComponent(id)}`,
  /** A rendition sized for a screen of `w`×`h` physical pixels. */
  render: (id: string, w: number, h: number, mode: "cover" | "contain") =>
    `${BASE}render/${encodeURIComponent(id)}?w=${Math.round(w)}&h=${Math.round(h)}&mode=${mode}`,
};

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  library: () => invoke<LibraryView>("get_library"),
  patch: (patch: DeepPartial<Settings> | Record<string, unknown>) => invoke<Settings>("patch_settings", { patch }),
  next: (display?: string) => invoke<void>("next_painting", { display }),
  previous: (display?: string) => invoke<void>("previous_painting", { display }),
  show: (id: string, display?: string) => invoke<void>("show_painting", { id, display }),
  openViewer: (id?: string, display?: string) => invoke<void>("open_viewer", { id, display }),
  openSettings: (section?: string) => invoke<void>("open_settings", { section }),
  closeViewer: () => invoke<void>("close_viewer"),
  setPlatePosition: (scope: "wallpaper" | "viewer", display: string | null, x: number, y: number) =>
    invoke<void>("set_plate_position", { scope, display, x, y }),
  setArrange: (on: boolean) => invoke<void>("set_arrange", { on }),
  setLibraryPath: (path: string | null) => invoke<void>("set_library_path", { path }),
  rescan: () => invoke<void>("rescan_library"),
  setExcluded: (id: string, excluded: boolean) => invoke<void>("set_excluded", { id, excluded }),
  setFavorite: (id: string, favorite: boolean) => invoke<void>("set_favorite", { id, favorite }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  reveal: (id: string) => invoke<void>("reveal_painting", { id }),
  colors: (id: string) => invoke<Colors>("painting_colors", { id }),
  reportImageError: (id: string, display?: string) => invoke<void>("report_image_error", { id, display }),
  openStylesFolder: () => invoke<void>("open_styles_folder"),
  createCustomStyle: (name: string) => invoke<string>("create_custom_style", { name }),
  openStyleFile: (id: string) => invoke<void>("open_style_file", { id }),
  restoreOriginalWallpaper: () => invoke<void>("restore_original_wallpaper"),
  refreshEnrichment: (id?: string) => invoke<void>("refresh_enrichment", { id }),
  clearImageCache: () => invoke<number>("clear_image_cache"),
  quit: () => invoke<void>("quit"),
  log: (message: string, level: "info" | "warn" | "error" = "info") => invoke<void>("frontend_log", { level, message }),
};

export function onSnapshot(cb: (s: Snapshot) => void): Promise<UnlistenFn> {
  return listen<Snapshot>("snapshot", (e) => cb(e.payload));
}

/** Inject (or update) the user's custom plate styles. */
export function injectCustomStyles(styles: { id: string; css: string }[]) {
  let el = document.getElementById("galeria-custom-styles") as HTMLStyleElement | null;
  if (!el) {
    el = document.createElement("style");
    el.id = "galeria-custom-styles";
    document.head.appendChild(el);
  }
  const css = styles.map((s) => `/* ${s.id} */\n${s.css}`).join("\n\n");
  if (el.textContent !== css) el.textContent = css;
}

/** `galeria --debug-pointer [down|up:]x,y` → synthetic pointer events (testing). */
export function installDebugPointer() {
  void listen<[string, number, number]>("debug-pointer", (e) => {
    const [kind, fx, fy] = e.payload;
    const clientX = fx * innerWidth;
    const clientY = fy * innerHeight;
    const type = kind === "down" ? "pointerdown" : kind === "up" ? "pointerup" : "pointermove";
    const target = (kind === "down" && document.elementFromPoint(clientX, clientY)) || window;
    target.dispatchEvent(new PointerEvent(type, { clientX, clientY, pointerId: 1, button: 0, bubbles: true }));
  });
}

export function debounce<A extends unknown[]>(fn: (...a: A) => void, ms: number) {
  let t: ReturnType<typeof setTimeout> | undefined;
  return (...a: A) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...a), ms);
  };
}
