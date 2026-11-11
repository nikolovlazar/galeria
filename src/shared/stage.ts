import { api, imageUrl } from "./api";
import { computeLayout, type Layout } from "./layout";
import { PanDriver } from "./pan";
import type { Painting, WallpaperSettings } from "./types";

interface Current {
  id: string;
  key: string;
  layer: HTMLElement;
  art: HTMLElement;
  pan: PanDriver | null;
  layout: Layout;
}

/**
 * Renders paintings into a stage with crossfades, wall/matte, frames and the
 * slow pan. Used by the wallpaper windows.
 */
export class Stage {
  private token = 0;
  private current: Current | null = null;

  constructor(
    private root: HTMLElement,
    private onError: (id: string) => void,
    /** Called when a new painting has actually appeared (after it decoded). */
    private onShown: (id: string) => void = () => {},
  ) {}

  get layout() {
    return this.current?.layout ?? null;
  }

  /** Id of the painting currently on screen (not merely requested). */
  get shownId() {
    return this.current?.id ?? null;
  }

  /** `shownAt` (unix seconds) anchors the pan phase so restarts don't jump back to the edge. */
  async show(p: Painting, w: WallpaperSettings, topInset: number, shownAt = 0) {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const key = JSON.stringify([
      p.id,
      vw,
      vh,
      topInset,
      w.fit,
      w.cropTolerance,
      w.pan,
      w.panSpeed,
      w.panPause,
      w.matte,
      w.matteColor,
      w.frame,
      w.fitMargin,
      w.shadow,
      w.spotlight,
    ]);
    if (this.current?.key === key) {
      this.setPaused(w.panPaused);
      return;
    }
    const token = ++this.token;
    const samePainting = this.current?.id === p.id;
    const layout = computeLayout(p.width, p.height, vw, vh, w, topInset);
    const dpr = window.devicePixelRatio || 1;
    const src = imageUrl.render(p.id, vw * dpr, vh * dpr, layout.kind === "contain" ? "contain" : "cover");

    const img = new Image();
    img.decoding = "async";
    img.src = src;
    try {
      await img.decode();
    } catch {
      if (token === this.token) this.onError(p.id);
      return;
    }
    if (token !== this.token) return;

    let wallColor = w.matteColor;
    if (layout.kind === "contain" && w.matte === "average") {
      try {
        wallColor = (await api.colors(p.id)).wall;
      } catch {
        /* keep the configured colour */
      }
      if (token !== this.token) return;
    }

    const layer = document.createElement("div");
    layer.className = "layer";

    if (layout.kind === "contain") {
      const wall = document.createElement("div");
      wall.className = "wall";
      wall.style.background = wallColor;
      layer.append(wall);
      if (w.matte === "blur") {
        const blur = new Image();
        blur.className = "wall-blur";
        blur.src = imageUrl.thumb(p.id);
        wall.append(blur);
      }
      if (w.spotlight && w.matte !== "blur") {
        const s = document.createElement("div");
        s.className = "spotlight";
        layer.append(s);
      }
    }

    const art = document.createElement("div");
    art.className = "art";
    const f = layout.frame;
    if (layout.kind === "contain") {
      if (w.frame !== "none") art.classList.add(`frame-${w.frame}`);
      else if (w.shadow) art.classList.add("shadow");
    }
    art.style.width = `${layout.w + 2 * f}px`;
    art.style.height = `${layout.h + 2 * f}px`;
    art.style.left = `${layout.x - f}px`;
    art.style.top = `${layout.y - f}px`;
    art.style.padding = `${f}px`;
    art.append(img);
    layer.append(art);
    this.root.append(layer);

    let pan: PanDriver | null = null;
    if (layout.kind === "pan" && layout.axis && layout.travel >= 1) {
      const prev = this.current;
      // Re-layout of the same painting keeps its place in the sweep; a fresh
      // painting starts from when it appeared.
      let elapsed = shownAt > 0 ? Math.max(0, Date.now() - shownAt * 1000) : 0;
      if (samePainting && prev?.pan) {
        elapsed = prev.pan.progress() * PanDriver.cycleMs(layout, w.panSpeed, w.panPause);
      }
      pan = new PanDriver(art, layout, w.panSpeed, w.panPause, elapsed);
      if (!w.panPaused) pan.play();
      else pan.pause();
    }

    const old = this.current;
    this.current = { id: p.id, key, layer, art, pan, layout };
    if (!samePainting) this.onShown(p.id);

    // Crossfade with a Web Animation (independent of requestAnimationFrame, which
    // WebKit may throttle for windows behind others), then pin the final state
    // and drop the animation so nothing keeps WebKit's animation timeline busy.
    const fade = old ? (samePainting ? 350 : w.transitionSeconds * 1000) : Math.min(800, w.transitionSeconds * 1000);
    let fadeAnim: Animation | null = null;
    const settle = () => {
      layer.classList.add("visible");
      fadeAnim?.cancel();
      fadeAnim = null;
      if (old && old.layer.isConnected) {
        old.pan?.stop();
        old.layer.remove();
      }
    };
    if (fade > 0) {
      fadeAnim = layer.animate([{ opacity: 0 }, { opacity: 1 }], { duration: fade, easing: "ease-in-out", fill: "forwards" });
      void fadeAnim.finished.then(settle, settle);
      setTimeout(settle, fade + 600);
    } else {
      settle();
    }
  }

  setPaused(paused: boolean) {
    const d = this.current?.pan;
    if (!d) return;
    if (paused && d.playing) d.pause();
    if (!paused && !d.playing) d.play();
  }

  clear() {
    this.token++;
    this.current?.pan?.stop();
    this.current?.layer.remove();
    this.current = null;
  }

  debugState() {
    const c = this.current;
    if (!c) return { painting: null };
    return {
      painting: c.id,
      layout: { kind: c.layout.kind, w: Math.round(c.layout.w), h: Math.round(c.layout.h), axis: c.layout.axis, travel: Math.round(c.layout.travel) },
      pan: c.pan ? { playing: c.pan.playing, progress: Number(c.pan.progress().toFixed(4)), cycleSeconds: Math.round(c.pan.cycleMs / 1000) } : null,
      transform: getComputedStyle(c.art).transform,
      layers: this.root.childElementCount,
      layerOpacity: getComputedStyle(c.layer).opacity,
      animations: document.getAnimations().length,
    };
  }
}
