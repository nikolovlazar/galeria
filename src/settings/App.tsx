import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { injectCustomStyles } from "../shared/api";
import { Browse, General, Library, NowShowing, Plate, Rotation, Wallpaper } from "./sections";
import { useSnapshot } from "./ui";

const NAV = [
  { id: "now", label: "Now Showing", icon: "◐" },
  { id: "browse", label: "Browse", icon: "▦" },
  { id: "wallpaper", label: "Wallpaper", icon: "▭" },
  { id: "plate", label: "Info Plate", icon: "▤" },
  { id: "rotation", label: "Rotation", icon: "↻" },
  { id: "library", label: "Library", icon: "❏" },
  { id: "general", label: "General", icon: "⚙︎" },
] as const;
type Tab = (typeof NAV)[number]["id"];

const fromHash = (): Tab => {
  const h = location.hash.replace("#", "");
  return (NAV.find((n) => n.id === h)?.id ?? "now") as Tab;
};

export function App() {
  const snap = useSnapshot();
  const [tab, setTab] = useState<Tab>(fromHash);

  useEffect(() => {
    const un = listen<string>("navigate", (e) => setTab((NAV.find((n) => n.id === e.payload)?.id ?? "now") as Tab));
    return () => void un.then((f) => f());
  }, []);
  useEffect(() => {
    history.replaceState(null, "", `#${tab}`);
  }, [tab]);
  useEffect(() => {
    if (snap) injectCustomStyles(snap.customStyles);
  }, [snap?.customStyles]); // eslint-disable-line react-hooks/exhaustive-deps

  const issues = snap ? snap.library.missing + snap.library.invalid + snap.library.duplicates : 0;

  return (
    <div className={`app platform-${snap?.platform ?? "macos"}`}>
      <nav className="sidebar">
        <div className="titlebar" data-tauri-drag-region />
        <div className="brand" data-tauri-drag-region>
          <img src="/galeria.png" alt="" />
          <div>
            <div className="brand-name">Galeria</div>
            <div className="brand-sub">{snap ? `${snap.library.valid} paintings` : "…"}</div>
          </div>
        </div>
        {NAV.map((n) => (
          <button key={n.id} type="button" className={`nav${tab === n.id ? " active" : ""}`} onClick={() => setTab(n.id)}>
            <span className="nav-icon">{n.icon}</span>
            {n.label}
            {n.id === "library" && issues > 0 && <span className="pill">{issues}</span>}
          </button>
        ))}
      </nav>
      <main className={`content tab-${tab}`}>
        <div className="titlebar" data-tauri-drag-region />
        {!snap ? (
          <div className="muted pad">Loading…</div>
        ) : (
          <div className="content-inner">
            {tab === "now" && <NowShowing snap={snap} />}
            {tab === "browse" && <Browse snap={snap} />}
            {tab === "wallpaper" && <Wallpaper snap={snap} />}
            {tab === "plate" && <Plate snap={snap} />}
            {tab === "rotation" && <Rotation snap={snap} />}
            {tab === "library" && <Library snap={snap} />}
            {tab === "general" && <General snap={snap} />}
          </div>
        )}
      </main>
    </div>
  );
}
