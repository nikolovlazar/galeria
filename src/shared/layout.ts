import type { FrameStyle, WallpaperSettings } from "./types";

export type LayoutKind = "cover" | "contain" | "pan";

export interface Layout {
  kind: LayoutKind;
  /** Painting size and position in CSS px (for pan: position at the start edge). */
  w: number;
  h: number;
  x: number;
  y: number;
  /** Pan axis and distance (px) for `pan`. */
  axis: "x" | "y" | null;
  travel: number;
  /** Fraction of the long side that a cover fit would crop. */
  crop: number;
  /** Frame thickness in px (contain only). */
  frame: number;
}

const FRAME_THICKNESS: Record<FrameStyle, number> = {
  none: 0,
  black: 0.011,
  white: 0.011,
  gilded: 0.03,
  walnut: 0.022,
};

/** Mirrors `layout_kind` in src-tauri/src/core.rs. */
export function layoutKind(iw: number, ih: number, vw: number, vh: number, w: Pick<WallpaperSettings, "fit" | "cropTolerance" | "pan">): LayoutKind {
  const cover = Math.max(vw / iw, vh / ih);
  const contain = Math.min(vw / iw, vh / ih);
  const crop = 1 - contain / cover;
  let kind: LayoutKind =
    w.fit === "fit" ? "contain" : w.fit === "fill" ? "cover" : w.fit === "pan" ? "pan" : crop <= w.cropTolerance + 1e-6 ? "cover" : w.pan ? "pan" : "contain";
  if (kind === "pan" && crop < 0.004) kind = "cover";
  return kind;
}

export function computeLayout(
  iw: number,
  ih: number,
  vw: number,
  vh: number,
  w: Pick<WallpaperSettings, "fit" | "cropTolerance" | "pan" | "fitMargin" | "frame">,
  topInset = 0,
): Layout {
  const cover = Math.max(vw / iw, vh / ih);
  const containScale = Math.min(vw / iw, vh / ih);
  const crop = 1 - containScale / cover;
  const kind = layoutKind(iw, ih, vw, vh, w);

  if (kind === "contain") {
    const frame = Math.round(Math.min(vw, vh) * FRAME_THICKNESS[w.frame]);
    const areaH = vh - topInset;
    const availW = vw * (1 - 2 * w.fitMargin) - 2 * frame;
    const availH = areaH * (1 - 2 * w.fitMargin) - 2 * frame;
    const s = Math.min(availW / iw, availH / ih);
    const pw = iw * s;
    const ph = ih * s;
    return { kind, w: pw, h: ph, x: (vw - pw) / 2, y: topInset + (areaH - ph) / 2, axis: null, travel: 0, crop, frame };
  }

  const pw = iw * cover;
  const ph = ih * cover;
  const axis = pw - vw > ph - vh ? "x" : "y";
  if (kind === "pan") {
    const travel = axis === "x" ? pw - vw : ph - vh;
    return {
      kind,
      w: pw,
      h: ph,
      x: axis === "x" ? 0 : (vw - pw) / 2,
      y: axis === "y" ? 0 : (vh - ph) / 2,
      axis,
      travel,
      crop,
      frame: 0,
    };
  }
  return { kind, w: pw, h: ph, x: (vw - pw) / 2, y: (vh - ph) / 2, axis: null, travel: 0, crop, frame: 0 };
}

