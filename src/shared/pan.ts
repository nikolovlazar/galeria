import type { Layout } from "./layout";

const MIN_STEP_MS = 33;

/** CSS `ease-in-out` = cubic-bezier(0.42, 0, 0.58, 1). */
function easeInOut(x: number) {
  const x1 = 0.42;
  const x2 = 0.58;
  // Solve bezierX(s) = x for s (Newton with bisection fallback), then return bezierY(s).
  const bx = (s: number) => 3 * x1 * s * (1 - s) ** 2 + 3 * x2 * s * s * (1 - s) + s ** 3;
  const by = (s: number) => 3 * 0 * s * (1 - s) ** 2 + 3 * 1 * s * s * (1 - s) + s ** 3;
  let lo = 0;
  let hi = 1;
  let s = x;
  for (let i = 0; i < 24; i++) {
    const v = bx(s) - x;
    if (Math.abs(v) < 1e-6) break;
    if (v > 0) hi = s;
    else lo = s;
    s = (lo + hi) / 2;
  }
  return by(s);
}

/**
 * Slow edge-to-edge ping-pong pan: rest, glide, rest, glide back.
 *
 * Instead of an infinite CSS/Web Animation (which keeps WebKit's WebContent
 * process busy every frame — measured 6–9% CPU, even while paused), this moves
 * the layer one device pixel at a time and sleeps in between: a few cheap
 * transform updates per second, and nothing at all while resting or paused.
 */
export class PanDriver {
  private timer: ReturnType<typeof setTimeout> | undefined;
  private start = 0; // wall-clock ms the timeline started
  private frozenAt: number | null = null; // elapsed ms when paused
  private last = NaN;
  readonly travelMs: number;
  readonly pauseMs: number;
  readonly cycleMs: number;

  static cycleMs(layout: Layout, speed: number, pauseSeconds: number) {
    return 2 * ((layout.travel / Math.max(0.1, speed)) * 1000 + Math.max(0, pauseSeconds) * 1000);
  }

  constructor(
    private el: HTMLElement,
    private layout: Layout,
    speed: number,
    pauseSeconds: number,
    /** Where in the cycle to start (ms). */
    elapsed: number,
  ) {
    this.travelMs = (layout.travel / Math.max(0.1, speed)) * 1000;
    this.pauseMs = Math.max(0, pauseSeconds) * 1000;
    this.cycleMs = 2 * (this.travelMs + this.pauseMs);
    this.start = Date.now() - (elapsed % this.cycleMs);
    this.apply(this.offsetAt(this.elapsed()));
  }

  elapsed() {
    return this.frozenAt ?? Date.now() - this.start;
  }

  /** Fraction (0..1) through the cycle, for carrying progress across re-layouts. */
  progress() {
    return (this.elapsed() % this.cycleMs) / this.cycleMs;
  }

  /** Offset in px (≤ 0) along the pan axis at `t` ms into the timeline. */
  offsetAt(t: number) {
    const c = ((t % this.cycleMs) + this.cycleMs) % this.cycleMs;
    const { pauseMs: p, travelMs: m } = this;
    const end = -this.layout.travel;
    if (c < p) return 0;
    if (c < p + m) return end * easeInOut((c - p) / m);
    if (c < 2 * p + m) return end;
    return end * (1 - easeInOut((c - 2 * p - m) / m));
  }

  private apply(v: number) {
    const dpr = window.devicePixelRatio || 1;
    const snapped = Math.round(v * dpr) / dpr;
    if (snapped === this.last) return;
    this.last = snapped;
    this.el.style.transform =
      this.layout.axis === "x" ? `translate3d(${snapped}px, 0, 0)` : `translate3d(0, ${snapped}px, 0)`;
  }

  private tick = () => {
    const t = this.elapsed();
    this.apply(this.offsetAt(t));
    // Sleep until the image would have moved by one device pixel.
    const dpr = window.devicePixelRatio || 1;
    const probe = 200;
    const v = Math.abs(this.offsetAt(t + probe) - this.offsetAt(t)) / probe; // px per ms
    let wait = v > 0 ? 1 / (v * dpr) : 1000;
    // Resting at an edge: wake up when the glide starts.
    const c = t % this.cycleMs;
    const { pauseMs: p, travelMs: m } = this;
    if (v === 0) {
      const restEnd = c < p ? p : c < 2 * p + m && c >= p + m ? 2 * p + m : c + 1000;
      wait = Math.max(16, restEnd - c);
    }
    // ≤30 updates/s: each one costs WebKit ~2 ms, and at wallpaper speeds 30 Hz
    // of whole-pixel steps is indistinguishable from continuous motion.
    this.timer = setTimeout(this.tick, Math.min(1000, Math.max(MIN_STEP_MS, wait)));
  };

  play() {
    if (this.frozenAt !== null) {
      this.start = Date.now() - this.frozenAt;
      this.frozenAt = null;
    }
    clearTimeout(this.timer);
    this.tick();
  }

  pause() {
    if (this.frozenAt === null) this.frozenAt = Date.now() - this.start;
    clearTimeout(this.timer);
    this.timer = undefined;
  }

  get playing() {
    return this.timer !== undefined;
  }

  stop() {
    clearTimeout(this.timer);
    this.timer = undefined;
  }
}
