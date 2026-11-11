import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Candidate, Catalogue, CatalogProgress, Colors, DeepPartial, DiscoverPage, DiscoverQuery, Job, LibraryView, Settings, Snapshot } from "./types";

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
  /** Licensing rules, genres and local catalogue status. */
  catalogue: () => invoke<Catalogue>("get_catalogue"),
  discover: (query: DiscoverQuery) => invoke<DiscoverPage>("discover_search", { query }),
  /** Download the open dataset a source is searched from (National Gallery of Art). */
  prepareCatalog: (source: string) => invoke<number>("discover_prepare", { source }),
  download: (items: Candidate[]) =>
    invoke<number>("download_start", {
      items: items.map((c) => ({ source: c.source, locator: c.locator, id: c.id, title: c.title, artist: c.artist, thumbUrl: c.thumbUrl })),
    }),
  cancelDownload: (id?: string) => invoke<void>("download_cancel", { id }),
  clearDownloads: () => invoke<void>("download_clear"),
  downloads: () => invoke<Job[]>("get_downloads"),
  /** Drop manifest entries whose file is gone. */
  cleanupManifest: () => invoke<number>("manifest_cleanup"),
  reportTheme: (dark: boolean) => invoke<void>("report_theme", { dark }),
  quit: () => invoke<void>("quit"),
  log: (message: string, level: "info" | "warn" | "error" = "info") => invoke<void>("frontend_log", { level, message }),
};

export function onSnapshot(cb: (s: Snapshot) => void): Promise<UnlistenFn> {
  return listen<Snapshot>("snapshot", (e) => cb(e.payload));
}

export function onDownloads(cb: (jobs: Job[]) => void): Promise<UnlistenFn> {
  return listen<Job[]>("downloads", (e) => cb(e.payload));
}

export function onCatalogProgress(cb: (p: CatalogProgress) => void): Promise<UnlistenFn> {
  return listen<CatalogProgress>("catalog-progress", (e) => cb(e.payload));
}

/** Tell the core when the system switches between light and dark (it also checks by itself). */
export function watchSystemTheme() {
  const mq = matchMedia("(prefers-color-scheme: dark)");
  const report = () => void api.reportTheme(mq.matches).catch(() => {});
  mq.addEventListener("change", report);
  report();
}

/**
 * `galeria --ui click:<selector>` / `--ui type:<selector>=<text>` / `--ui key:<selector>=Enter`:
 * drive the settings window from the command line (testing without a mouse).
 */
export function installDebugUi() {
  void listen<string>("debug-ui", (e) => {
    const [kind, ...rest] = e.payload.split(":");
    const arg = rest.join(":");
    if (kind === "click") {
      const el = document.querySelector<HTMLElement>(arg);
      void api.log(`ui click ${arg}: ${el ? "ok" : "not found"}`);
      el?.click();
    } else if (kind === "type" || kind === "key") {
      // The selector may itself contain "=" (`[data-test=x]=text`).
      const eq = arg.includes("]=") ? arg.lastIndexOf("]=") + 1 : arg.indexOf("=");
      const sel = arg.slice(0, eq);
      const text = arg.slice(eq + 1);
      const el = document.querySelector<HTMLInputElement>(sel);
      void api.log(`ui ${kind} ${sel}: ${el ? "ok" : "not found"}`);
      if (!el) return;
      if (kind === "type") {
        // React tracks the value through the native setter.
        Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), "value")?.set?.call(el, text);
        el.dispatchEvent(new Event("input", { bubbles: true }));
        el.dispatchEvent(new Event("change", { bubbles: true }));
      } else {
        el.dispatchEvent(new KeyboardEvent("keydown", { key: text, bubbles: true }));
      }
    } else if (kind === "dump") {
      const els = [...document.querySelectorAll<HTMLElement>(arg || "[data-test]")];
      void api.log(`ui dump ${arg}: ${els.length} → ${els.slice(0, 40).map((x) => (x.dataset.test ?? x.className) + "=" + (x.innerText || "").replace(/\s+/g, " ").slice(0, 80)).join(" | ")}`);
    }
  });
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
