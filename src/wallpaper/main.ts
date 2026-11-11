import "../shared/base.css";
import "../shared/plates.css";
import "./wallpaper.css";
import { listen } from "@tauri-apps/api/event";
import { api, debounce, injectCustomStyles, installDebugPointer, onSnapshot } from "../shared/api";
import { makeDraggable, placePlate, plateSignature, renderPlate, type Area } from "../shared/plate";
import { Stage } from "../shared/stage";
import type { Display, Painting, PlatePosition, Snapshot } from "../shared/types";

const displayKey = new URLSearchParams(location.search).get("display") ?? "";
const stageEl = document.getElementById("stage")!;
const stage = new Stage(
  stageEl,
  (id) => void api.reportImageError(id, displayKey),
  () => renderPlateLayer(),
);

let snap: Snapshot | null = null;
let plate: HTMLElement | null = null;
let plateSig = "";
let platePaintingId: string | null = null;
let hideTimer: ReturnType<typeof setTimeout> | undefined;
let lastShown = false;
let lastPlatePainting: Painting | null = null;
let dragging = false;

const hud = document.createElement("div");
hud.className = "arrange-hud";
hud.innerHTML = `
  <div class="arrange-title">Arrange the info plate</div>
  <div class="arrange-hint">Drag it anywhere — it snaps to edges and centre lines.</div>
  <div class="arrange-actions">
    <button data-act="toggle"></button>
    <button data-act="reset">Reset</button>
    <button data-act="done" class="primary">Done</button>
  </div>`;
document.body.append(hud);
hud.addEventListener("click", (e) => {
  const act = (e.target as HTMLElement).closest("button")?.dataset.act;
  if (!snap) return;
  if (act === "done") void api.setArrange(false);
  if (act === "toggle") void api.patch({ plate: { visible: !snap.settings.plate.visible } });
  if (act === "reset") {
    const d = snap.settings.plate.defaultPosition;
    void api.setPlatePosition("wallpaper", displayKey, d.x, d.y);
  }
});

const guides = document.createElement("div");
guides.className = "guides";
guides.innerHTML = `<i class="gx"></i><i class="gy"></i>`;
document.body.append(guides);

function display(): Display | undefined {
  return snap?.displays.find((d) => d.key === displayKey) ?? snap?.displays[0];
}

function plateArea(): Area {
  const d = display();
  const m = snap?.settings.plate.margin ?? 36;
  const ins = d?.insets ?? { top: 0, right: 0, bottom: 0, left: 0 };
  return {
    x: ins.left + m,
    y: ins.top + m * 0.6,
    w: window.innerWidth - ins.left - ins.right - 2 * m,
    h: window.innerHeight - ins.top - ins.bottom - m * 1.6,
  };
}

function platePosition(): PlatePosition {
  const ps = snap!.settings.plate;
  return ps.positions[displayKey] ?? ps.defaultPosition;
}

function knownStyle(id: string) {
  return !!snap && (snap.builtinStyles.some((s) => s.id === id) || snap.customStyles.some((s) => s.id === id));
}

function renderPlateLayer() {
  if (!snap) return;
  // The plate describes what is actually on screen: while the next painting is
  // still loading, keep showing the current one's label.
  const wanted = display()?.painting ?? null;
  const p = wanted && wanted.id === stage.shownId ? wanted : stage.shownId ? lastPlatePainting : null;
  if (wanted && wanted.id === stage.shownId) lastPlatePainting = wanted;
  const ps = { ...snap.settings.plate };
  if (!knownStyle(ps.style)) ps.style = "plastic";
  const sig = plateSignature(p, ps);
  if (sig !== plateSig) {
    plateSig = sig;
    plate?.remove();
    plate = p ? renderPlate(p, ps) : null;
    if (plate) document.body.insertBefore(plate, hud);
  }
  if (!plate) return;

  const arranging = snap.arrange;
  const show = ps.visible || arranging;
  plate.classList.toggle("ghost", arranging && !ps.visible);

  // Auto-hide a while after a new painting appears (or the plate is re-shown).
  const reveal = (p && p.id !== platePaintingId) || (show && !lastShown);
  platePaintingId = p?.id ?? null;
  lastShown = show;
  if (reveal) {
    clearTimeout(hideTimer);
    plate.classList.remove("hidden");
    if (ps.autoHideSeconds > 0) {
      hideTimer = setTimeout(() => {
        if (!snap?.arrange) plate?.classList.add("hidden");
      }, ps.autoHideSeconds * 1000);
    }
  }
  if (!show) plate.classList.add("hidden");
  else if (arranging || ps.autoHideSeconds === 0) plate.classList.remove("hidden");

  if (!dragging) placePlate(plate, platePosition(), plateArea());
}

function render() {
  if (!snap) return;
  const d = display();
  injectCustomStyles(snap.customStyles);
  document.body.classList.toggle("arranging", snap.arrange);
  const toggle = hud.querySelector<HTMLButtonElement>('[data-act="toggle"]');
  if (toggle) toggle.textContent = snap.settings.plate.visible ? "Hide plate" : "Show plate";

  if (d?.painting) {
    void stage.show(d.painting, snap.settings.wallpaper, d.insets.top, d.shownAt);
  } else {
    stage.clear();
  }
  renderPlateLayer();
}

makeDraggable(
  () => plate,
  plateArea,
  () => !!snap?.arrange,
  (pos) => {
    dragging = true;
    guides.classList.toggle("show-x", pos.x === 0.5);
    guides.classList.toggle("show-y", pos.y === 0.5);
  },
  (pos) => {
    dragging = false;
    guides.className = "guides";
    void api.setPlatePosition("wallpaper", displayKey, pos.x, pos.y);
  },
);

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && snap?.arrange) void api.setArrange(false);
});
window.addEventListener(
  "resize",
  debounce(() => render(), 250),
);
// Re-place the plate once web fonts/metrics settle.
document.fonts?.ready.then(() => renderPlateLayer());

onSnapshot((s) => {
  snap = s;
  render();
});
installDebugPointer();
void listen("debug-dump", () => {
  void api.log(
    JSON.stringify({
      display: displayKey,
      visibility: document.visibilityState,
      viewport: [innerWidth, innerHeight, devicePixelRatio],
      stage: stage.debugState(),
      plate: plate ? { title: plate.querySelector(".plate-title")?.textContent, painting: lastPlatePainting?.id, style: plate.dataset.style, left: plate.style.left, top: plate.style.top, w: plate.offsetWidth, h: plate.offsetHeight, hidden: plate.classList.contains("hidden") } : null,
    }),
  );
});
api.snapshot().then((s) => {
  snap = s;
  render();
});
