import "../shared/base.css";
import "../shared/plates.css";
import "./viewer.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { api, imageUrl, injectCustomStyles, installDebugPointer, onSnapshot } from "../shared/api";
import { makeDraggable, placePlate, plateSignature, renderPlate, type Area } from "../shared/plate";
import type { Painting, Snapshot } from "../shared/types";

const params = new URLSearchParams(location.search);
let currentId = params.get("id") ?? "";
const displayKey = params.get("display") ?? undefined;
const win = getCurrentWindow();

const stage = document.getElementById("stage")!;
const canvas = document.createElement("div");
canvas.className = "viewer-canvas";
const lowres = new Image();
lowres.className = "viewer-img lowres";
const hires = new Image();
hires.className = "viewer-img hires";
canvas.append(lowres, hires);
stage.append(canvas);

const minimap = document.createElement("div");
minimap.className = "minimap";
const miniView = document.createElement("div");
miniView.className = "minimap-view";
minimap.append(miniView);
document.body.append(minimap);

const toast = document.createElement("div");
toast.className = "toast";
document.body.append(toast);

const help = document.createElement("div");
help.className = "help";
help.innerHTML = `
  <h2>Viewer</h2>
  <dl>
    <dt>Move the mouse</dt><dd>Reveal the painting edge to edge</dd>
    <dt>Scroll / pinch</dt><dd>Zoom in and out</dd>
    <dt>Double-click · F</dt><dd>Fit the whole painting / fill the screen</dd>
    <dt>0</dt><dd>Reset zoom</dd>
    <dt>← →</dt><dd>Previous / next painting</dd>
    <dt>I</dt><dd>Show or hide the info plate (drag it anywhere)</dd>
    <dt>W</dt><dd>Use as desktop wallpaper</dd>
    <dt>O</dt><dd>Open the museum page</dd>
    <dt>?</dt><dd>This help</dd>
    <dt>Esc</dt><dd>Close the viewer</dd>
  </dl>`;
document.body.append(help);

let snap: Snapshot | null = null;
let library: Painting[] = [];
let painting: Painting | null = null;
let plate: HTMLElement | null = null;
let plateSig = "";
let plateVisible = true;

// View state ---------------------------------------------------------------
let fitMode = false;
let zoom = 1;
const mouse = { x: 0.5, y: 0.5 };
const cur = { x: 0, y: 0, s: 1 };
const target = { x: 0, y: 0, s: 1 };
let raf = 0;
let last = 0;
let first = true;
let minimapTimer: ReturnType<typeof setTimeout> | undefined;

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

function scales() {
  if (!painting) return { cover: 1, contain: 1 };
  const vw = innerWidth;
  const vh = innerHeight;
  return {
    cover: Math.max(vw / painting.width, vh / painting.height),
    contain: Math.min(vw / painting.width, vh / painting.height),
  };
}

function zoomLimits() {
  const { cover, contain } = scales();
  const base = fitMode ? contain : cover;
  const pixelPerfect = 1 / (window.devicePixelRatio || 1);
  return { min: Math.min(1, contain / base), max: Math.max(2, (pixelPerfect * 2.5) / base), base };
}

function computeTarget() {
  if (!painting) return;
  const { base } = zoomLimits();
  const s = base * zoom;
  const w = painting.width * s;
  const h = painting.height * s;
  const vw = innerWidth;
  const vh = innerHeight;
  const dz = snap?.settings.viewer.edgeDeadZone ?? 0.08;
  const t = (m: number) => clamp((m - dz) / (1 - 2 * dz), 0, 1);
  target.s = s;
  target.x = w > vw ? -(w - vw) * t(mouse.x) : (vw - w) / 2;
  target.y = h > vh ? -(h - vh) * t(mouse.y) : (vh - h) / 2;
}

function apply() {
  if (!painting) return;
  const base = painting.width * scales().cover;
  const k = cur.s / scales().cover;
  canvas.style.width = `${base}px`;
  canvas.style.height = `${(base / painting.width) * painting.height}px`;
  canvas.style.transform = `translate3d(${cur.x}px, ${cur.y}px, 0) scale(${k})`;
  updateMinimap();
}

function tick(now: number) {
  const dt = last ? Math.min(64, now - last) : 16.7;
  last = now;
  const smooth = snap?.settings.viewer.smoothing ?? 0.6;
  const f = 1 - Math.pow(smooth, dt / 16.7);
  cur.x += (target.x - cur.x) * f;
  cur.y += (target.y - cur.y) * f;
  cur.s += (target.s - cur.s) * f;
  apply();
  const settled = Math.abs(target.x - cur.x) < 0.1 && Math.abs(target.y - cur.y) < 0.1 && Math.abs(target.s - cur.s) < 1e-5;
  if (settled) {
    Object.assign(cur, target);
    apply();
    raf = 0;
    last = 0;
  } else {
    raf = requestAnimationFrame(tick);
  }
}

function kick(immediate = false) {
  computeTarget();
  if (immediate) {
    Object.assign(cur, target);
    apply();
    return;
  }
  if (!raf) raf = requestAnimationFrame(tick);
}

function overflowing() {
  if (!painting) return false;
  return painting.width * target.s > innerWidth + 1 || painting.height * target.s > innerHeight + 1;
}

function updateMinimap() {
  if (!painting || !snap?.settings.viewer.minimap) return;
  const s = cur.s;
  const vx = clamp(-cur.x / (painting.width * s), 0, 1);
  const vy = clamp(-cur.y / (painting.height * s), 0, 1);
  const vw = clamp(innerWidth / (painting.width * s), 0, 1);
  const vh = clamp(innerHeight / (painting.height * s), 0, 1);
  miniView.style.left = `${vx * 100}%`;
  miniView.style.top = `${vy * 100}%`;
  miniView.style.width = `${vw * 100}%`;
  miniView.style.height = `${vh * 100}%`;
}

function pokeMinimap() {
  if (!snap?.settings.viewer.minimap || !overflowing()) return;
  minimap.classList.add("show");
  clearTimeout(minimapTimer);
  minimapTimer = setTimeout(() => minimap.classList.remove("show"), 1400);
}

function showToast(text: string, ms = 2200) {
  toast.textContent = text;
  toast.classList.add("show");
  setTimeout(() => toast.classList.remove("show"), ms);
}

// Painting -----------------------------------------------------------------

async function load(id: string) {
  const p = library.find((x) => x.id === id) ?? (snap?.displays.map((d) => d.painting).find((x) => x?.id === id) as Painting | undefined);
  if (!p) return;
  currentId = id;
  painting = p;
  zoom = 1;
  document.title = p.artist ? `${p.title} — ${p.artist}` : p.title;
  hires.classList.remove("loaded");
  lowres.src = imageUrl.thumb(p.id);
  const big = imageUrl.original(p.id);
  hires.src = big;
  // Size the miniature to the painting's aspect.
  const mw = 200;
  minimap.style.width = `${mw}px`;
  minimap.style.height = `${(mw / p.width) * p.height}px`;
  minimap.style.backgroundImage = `url("${imageUrl.thumb(p.id)}")`;
  kick(true);
  renderPlateLayer();
  try {
    await hires.decode();
    if (hires.src === big) hires.classList.add("loaded");
  } catch {
    showToast("Could not load the full-resolution image");
  }
}

function neighbour(dir: 1 | -1) {
  const list = library.filter((p) => !p.excluded);
  if (!list.length) return;
  const i = list.findIndex((p) => p.id === currentId);
  const next = list[(i + dir + list.length) % list.length];
  void load(next.id);
}

// Plate --------------------------------------------------------------------

function plateArea(): Area {
  const m = 28;
  return { x: m, y: m, w: innerWidth - 2 * m, h: innerHeight - 2 * m };
}

function renderPlateLayer() {
  if (!snap || !painting) return;
  const ps = { ...snap.settings.plate };
  if (![...snap.builtinStyles, ...snap.customStyles].some((s) => s.id === ps.style)) ps.style = "plastic";
  const sig = plateSignature(painting, ps);
  if (sig !== plateSig) {
    plateSig = sig;
    plate?.remove();
    plate = renderPlate(painting, ps);
    plate.classList.add("viewer-plate");
    document.body.append(plate);
  }
  plate!.classList.toggle("hidden", !plateVisible);
  placePlate(plate!, snap.settings.viewer.platePosition, plateArea());
}

makeDraggable(
  () => plate,
  plateArea,
  () => plateVisible,
  () => {},
  (pos) => void api.setPlatePosition("viewer", null, pos.x, pos.y),
);

// Input ----------------------------------------------------------------------

window.addEventListener("pointermove", (e) => {
  mouse.x = e.clientX / innerWidth;
  mouse.y = e.clientY / innerHeight;
  kick();
  pokeMinimap();
});

window.addEventListener(
  "wheel",
  (e) => {
    e.preventDefault();
    const { min, max } = zoomLimits();
    // Pinch arrives as ctrl+wheel with small deltas.
    const factor = Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0022));
    zoom = clamp(zoom * factor, min, max);
    kick();
    pokeMinimap();
  },
  { passive: false },
);

stage.addEventListener("dblclick", () => {
  fitMode = !fitMode;
  zoom = 1;
  kick();
});

window.addEventListener("keydown", (e) => {
  if (e.metaKey || e.ctrlKey) return;
  switch (e.key) {
    case "Escape":
      if (help.classList.contains("show")) help.classList.remove("show");
      else void api.closeViewer().catch(() => win.close());
      break;
    case "ArrowRight":
    case "ArrowDown":
      neighbour(1);
      break;
    case "ArrowLeft":
    case "ArrowUp":
      neighbour(-1);
      break;
    case "i":
    case "I":
      plateVisible = !plateVisible;
      void api.patch({ viewer: { showPlate: plateVisible } });
      renderPlateLayer();
      break;
    case "f":
    case "F":
      fitMode = !fitMode;
      zoom = 1;
      kick();
      break;
    case "0":
      zoom = 1;
      kick();
      break;
    case "+":
    case "=":
      zoom = clamp(zoom * 1.25, zoomLimits().min, zoomLimits().max);
      kick();
      break;
    case "-":
    case "_":
      zoom = clamp(zoom / 1.25, zoomLimits().min, zoomLimits().max);
      kick();
      break;
    case "w":
    case "W":
      if (painting) {
        void api.show(painting.id, displayKey);
        showToast("Set as desktop wallpaper");
      }
      break;
    case "o":
    case "O":
      if (painting?.sourcePage) void api.openUrl(painting.sourcePage);
      break;
    case "?":
    case "h":
    case "H":
      help.classList.toggle("show");
      break;
  }
});

window.addEventListener("resize", () => {
  kick(true);
  renderPlateLayer();
});

// Boot ---------------------------------------------------------------------

function applySnapshot(s: Snapshot) {
  snap = s;
  injectCustomStyles(s.customStyles);
  document.body.classList.toggle("hide-cursor", s.settings.viewer.hideCursor);
  if (first) {
    plateVisible = s.settings.viewer.showPlate;
  }
  if (painting) {
    const fresh = s.displays.map((d) => d.painting).find((p) => p?.id === painting!.id);
    if (fresh) painting = fresh;
    renderPlateLayer();
  }
}

async function boot() {
  const [s, lib] = await Promise.all([api.snapshot(), api.library()]);
  applySnapshot(s);
  library = lib.paintings;
  await load(currentId);
  first = false;
  showToast("Move to explore · scroll to zoom · ← → browse · I info · ? help · Esc close", 4200);
  onSnapshot(applySnapshot);
  listen<string>("viewer-show", (e) => void load(e.payload));
  installDebugPointer();
  listen("debug-dump", () => {
    void api.log(JSON.stringify({ painting: painting?.id, size: [painting?.width, painting?.height], fitMode, zoom, cur, target, hires: hires.classList.contains("loaded"), plate: plate ? { left: plate.style.left, top: plate.style.top, hidden: plate.classList.contains("hidden") } : null }));
  });
}

void boot();
