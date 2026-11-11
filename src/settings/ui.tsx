import { useEffect, useRef, useState, type ReactNode } from "react";
import { onSnapshot, api } from "../shared/api";
import type { LibraryView, Snapshot } from "../shared/types";

export function useSnapshot() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  useEffect(() => {
    let alive = true;
    api.snapshot().then((s) => alive && setSnap(s));
    const un = onSnapshot((s) => alive && setSnap(s));
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, []);
  return snap;
}

/** The full painting list, refetched whenever the library or its metadata changes. */
export function useLibrary(snap: Snapshot | null) {
  const [lib, setLib] = useState<LibraryView | null>(null);
  const key = snap
    ? JSON.stringify([snap.library.loadedAt, snap.library.enriched, snap.settings.excluded.length, snap.settings.favorites, snap.library.root])
    : "";
  useEffect(() => {
    if (!snap) return;
    let alive = true;
    api.library().then((l) => alive && setLib(l));
    return () => {
      alive = false;
    };
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps
  return lib;
}

export function Section({ title, description, children, actions }: { title: string; description?: ReactNode; children: ReactNode; actions?: ReactNode }) {
  return (
    <section className="section">
      <header className="section-head">
        <div>
          <h2>{title}</h2>
          {description && <p className="muted">{description}</p>}
        </div>
        {actions && <div className="section-actions">{actions}</div>}
      </header>
      <div className="card">{children}</div>
    </section>
  );
}

export function Row({ label, hint, children, disabled }: { label: ReactNode; hint?: ReactNode; children?: ReactNode; disabled?: boolean }) {
  return (
    <div className={`row${disabled ? " disabled" : ""}`}>
      <div className="row-label">
        <div>{label}</div>
        {hint && <div className="row-hint">{hint}</div>}
      </div>
      <div className="row-control">{children}</div>
    </div>
  );
}

export function Toggle({ checked, onChange, disabled }: { checked: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      className={`toggle${checked ? " on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

export function Segmented<T extends string | number>({ value, options, onChange }: { value: T; options: { value: T; label: ReactNode }[]; onChange: (v: T) => void }) {
  return (
    <div className="segmented">
      {options.map((o) => (
        <button key={String(o.value)} type="button" className={o.value === value ? "active" : ""} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** A slider that keeps local state while dragging and commits after a pause. */
export function Slider({
  value,
  min,
  max,
  step,
  onCommit,
  format,
}: {
  value: number;
  min: number;
  max: number;
  step: number;
  onCommit: (v: number) => void;
  format?: (v: number) => string;
}) {
  const [local, setLocal] = useState(value);
  const editing = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => {
    if (!editing.current) setLocal(value);
  }, [value]);
  return (
    <div className="slider">
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={local}
        onChange={(e) => {
          const v = Number(e.target.value);
          editing.current = true;
          setLocal(v);
          clearTimeout(timer.current);
          timer.current = setTimeout(() => {
            editing.current = false;
            onCommit(v);
          }, 280);
        }}
      />
      <output>{format ? format(local) : local}</output>
    </div>
  );
}

export function Select<T extends string | number>({ value, options, onChange }: { value: T; options: { value: T; label: string }[]; onChange: (v: T) => void }) {
  return (
    <select
      className="select"
      value={String(value)}
      onChange={(e) => {
        const o = options.find((o) => String(o.value) === e.target.value);
        if (o) onChange(o.value);
      }}
    >
      {options.map((o) => (
        <option key={String(o.value)} value={String(o.value)}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

export function Button({ children, onClick, kind, disabled, title }: { children: ReactNode; onClick?: () => void; kind?: "primary" | "danger" | "ghost"; disabled?: boolean; title?: string }) {
  return (
    <button type="button" className={`btn${kind ? ` btn-${kind}` : ""}`} onClick={onClick} disabled={disabled} title={title}>
      {children}
    </button>
  );
}

export function timeUntil(ts: number | null) {
  if (!ts) return null;
  const s = ts - Date.now() / 1000;
  if (s <= 60) return "in under a minute";
  const m = Math.round(s / 60);
  if (m < 60) return `in ${m} min`;
  const h = Math.floor(m / 60);
  return `in ${h} h ${m % 60} min`;
}

export function bytes(n: number) {
  if (n > 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n > 1e6) return `${(n / 1e6).toFixed(1)} MB`;
  return `${Math.round(n / 1e3)} KB`;
}
