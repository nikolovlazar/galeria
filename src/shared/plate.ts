import { renderSVG } from "uqr";
import type { Painting, PlatePosition, PlateSettings } from "./types";

export interface Area {
  x: number;
  y: number;
  w: number;
  h: number;
}

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text?: string | null) => {
  const e = document.createElement(tag);
  e.className = cls;
  if (text != null) e.textContent = text;
  return e;
};

const capitalize = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

const normalize = (s: string) =>
  s
    .toLowerCase()
    .replace(/\(.*?\)/g, "")
    .replace(/\b(courtesy|the|of|cc0|public domain|photo|image)\b/g, "")
    .replace(/[^a-z]/g, "");

/** Build the museum label for a painting. Pure DOM; styles live in plates.css. */
export function renderPlate(p: Painting, s: PlateSettings): HTMLElement {
  const f = s.fields;
  const plate = el("div", "plate");
  plate.dataset.style = s.style;
  plate.style.setProperty("--plate-scale", String(s.scale));
  plate.style.setProperty("--plate-width", `${s.width}px`);
  plate.style.setProperty("--plate-lines", s.descriptionLines > 0 ? String(s.descriptionLines) : "999");

  const body = el("div", "plate-body");
  const heading = el("div", "plate-heading");
  heading.append(el("span", "plate-title", p.title));
  if (f.date && p.date) heading.append(el("span", "plate-date", p.date));
  body.append(heading);
  if (p.originalTitle) body.append(el("div", "plate-original-title", p.originalTitle));

  if (f.artist && p.artist) body.append(el("div", "plate-artist", p.artist));
  if (f.artistBio && p.artistBio) body.append(el("div", "plate-bio", p.artistBio));

  const meta = el("div", "plate-meta");
  if (f.medium && p.medium) meta.append(el("span", "plate-medium", capitalize(p.medium)));
  if (f.dimensions && p.dimensions) meta.append(el("span", "plate-dimensions", p.dimensions));
  if (meta.childElementCount) body.append(meta);

  const showDescription = f.description && !!p.description && (s.foreignDescriptions || !p.descriptionLang || p.descriptionLang === "en");
  if (showDescription && p.description) {
    const d = el("p", "plate-description", p.description);
    if (p.descriptionLang) d.lang = p.descriptionLang;
    body.append(d);
  }

  const footer = el("div", "plate-footer");
  if (f.museum && p.museum) {
    const m = el("div", "plate-museum", p.museum);
    if (p.creditLine) m.append(el("span", "plate-credit-line", p.creditLine));
    footer.append(m);
  }
  if (f.accession && p.accession) footer.append(el("div", "plate-accession", p.accession));
  // An attribution license (CC BY, CC BY-SA) is honoured even when the credit
  // and license details are switched off: the author and license always show.
  const owed = p.attributionRequired;
  if ((f.credit || owed) && p.credit) {
    const redundant = f.museum && p.museum && normalize(p.credit) === normalize(p.museum);
    if (!redundant) footer.append(el("div", "plate-credit", p.credit));
  }
  const creditNamesLicense = !!p.credit && /\bCC[ -]BY/i.test(p.credit);
  if (p.license && (f.license || (owed && !creditNamesLicense))) footer.append(el("div", "plate-license", p.license));
  if (showDescription && p.descriptionSource) {
    const lang = p.descriptionLang && p.descriptionLang !== "en" ? ` (${langName(p.descriptionLang)})` : "";
    const lic = p.descriptionLicense ? `, ${p.descriptionLicense}` : "";
    footer.append(el("div", "plate-text-credit", `Text: ${p.descriptionSource}${lic}${lang}`));
  }
  if (footer.childElementCount) body.append(footer);
  plate.append(body);

  const qrTarget = p.sourcePage ?? p.wikipediaUrl;
  if (f.qr && qrTarget) {
    const qr = el("div", "plate-qr");
    qr.innerHTML = renderSVG(qrTarget, { border: 0, blackColor: "currentColor", whiteColor: "transparent", ecc: "M" });
    qr.title = qrTarget;
    plate.append(qr);
  }
  return plate;
}

function langName(code: string) {
  try {
    return new Intl.DisplayNames(["en"], { type: "language" }).of(code) ?? code;
  } catch {
    return code;
  }
}

/** Position a plate inside `area` from a normalized (0..1) position. */
export function placePlate(plate: HTMLElement, pos: PlatePosition, area: Area) {
  const w = plate.offsetWidth;
  const h = plate.offsetHeight;
  const left = area.x + pos.x * Math.max(0, area.w - w);
  const top = area.y + pos.y * Math.max(0, area.h - h);
  plate.style.left = `${Math.round(left)}px`;
  plate.style.top = `${Math.round(top)}px`;
}

const SNAPS = [0, 0.5, 1];

/**
 * Drag a plate around `area()`. Positions snap to edges/centre when close.
 * `enabled()` lets the caller gate dragging (e.g. wallpaper arrange mode).
 */
export function makeDraggable(
  plate: () => HTMLElement | null,
  area: () => Area,
  enabled: () => boolean,
  onMove: (pos: PlatePosition) => void,
  onDrop: (pos: PlatePosition) => void,
) {
  let drag: { dx: number; dy: number; id: number } | null = null;
  const toPos = (left: number, top: number, p: HTMLElement): PlatePosition => {
    const a = area();
    const fx = a.w - p.offsetWidth > 0 ? (left - a.x) / (a.w - p.offsetWidth) : 0;
    const fy = a.h - p.offsetHeight > 0 ? (top - a.y) / (a.h - p.offsetHeight) : 0;
    const snap = (v: number) => {
      const c = Math.min(1, Math.max(0, v));
      const near = SNAPS.find((s) => Math.abs(s - c) < 0.025);
      return near ?? c;
    };
    return { x: snap(fx), y: snap(fy) };
  };
  window.addEventListener("pointerdown", (e) => {
    const p = plate();
    if (!p || !enabled() || e.button !== 0) return;
    if (!(e.target instanceof Node) || !p.contains(e.target)) return;
    const r = p.getBoundingClientRect();
    drag = { dx: e.clientX - r.left, dy: e.clientY - r.top, id: e.pointerId };
    p.classList.add("dragging");
    e.preventDefault();
  });
  window.addEventListener("pointermove", (e) => {
    const p = plate();
    if (!drag || !p || e.pointerId !== drag.id) return;
    const pos = toPos(e.clientX - drag.dx, e.clientY - drag.dy, p);
    placePlate(p, pos, area());
    onMove(pos);
  });
  const end = (e: PointerEvent) => {
    const p = plate();
    if (!drag || !p || e.pointerId !== drag.id) return;
    const pos = toPos(e.clientX - drag.dx, e.clientY - drag.dy, p);
    drag = null;
    p.classList.remove("dragging");
    onDrop(pos);
  };
  window.addEventListener("pointerup", end);
  window.addEventListener("pointercancel", end);
}

/** A stable key for "does the plate need to be rebuilt". */
export function plateSignature(p: Painting | null | undefined, s: PlateSettings) {
  const { positions: _p, defaultPosition: _d, ...rest } = s;
  return JSON.stringify([p, rest]);
}
