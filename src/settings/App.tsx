import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { injectCustomStyles } from "../shared/api";
import { Discover, type DiscoverIntent } from "./Discover";
import { Sources } from "./Sources";
import { Browse, General, Library, NowShowing, Plate, Rotation, Wallpaper } from "./sections";
import { useSnapshot } from "./ui";

const NAV = [
  { id: "now", label: "Now Showing", icon: "◐" },
  { id: "browse", label: "Browse", icon: "▦" },
  { id: "discover", label: "Discover", icon: "✦" },
  { id: "sources", label: "Museums", icon: "⌂" },
  { id: "rotation", label: "Rotation", icon: "↻" },
  { id: "wallpaper", label: "Wallpaper", icon: "▭" },
  { id: "plate", label: "Info Plate", icon: "▤" },
  { id: "library", label: "Library", icon: "❏" },
  { id: "general", label: "General", icon: "⚙︎" },
] as const;
type Tab = (typeof NAV)[number]["id"];

const asTab = (id: string): Tab => (NAV.find((n) => n.id === id)?.id ?? "now") as Tab;
const fromHash = (): Tab => asTab(location.hash.replace("#", ""));

export function App() {
  const snap = useSnapshot();
  const [tab, setTab] = useState<Tab>(fromHash);
  // Where Discover should start when another tab sends the user there.
  const [intent, setIntent] = useState<{ n: number; value: DiscoverIntent }>({ n: 0, value: {} });

  useEffect(() => {
    const un = listen<string>("navigate", (e) => setTab(asTab(e.payload)));
    return () => void un.then((f) => f());
  }, []);
  useEffect(() => {
    history.replaceState(null, "", `#${tab}`);
  }, [tab]);
  useEffect(() => {
    if (snap) injectCustomStyles(snap.customStyles);
  }, [snap?.customStyles]); // eslint-disable-line react-hooks/exhaustive-deps

  const issues = snap ? snap.library.missing + snap.library.invalid + snap.library.duplicates : 0;
  const discover = (value: DiscoverIntent) => {
    setIntent((i) => ({ n: i.n + 1, value }));
    setTab("discover");
  };

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
          <button key={n.id} type="button" data-nav={n.id} className={`nav${tab === n.id ? " active" : ""}`} onClick={() => setTab(n.id)}>
            <span className="nav-icon">{n.icon}</span>
            {n.label}
            {n.id === "library" && issues > 0 && <span className="pill">{issues}</span>}
            {n.id === "discover" && snap && snap.downloads.active > 0 && <span className="pill busy">↓ {snap.downloads.active}</span>}
            {n.id === "sources" && snap && snap.museums.some((m) => !m.enabled) && <span className="pill quiet">{snap.museums.filter((m) => !m.enabled).length} off</span>}
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
            {tab === "discover" && <Discover key={intent.n} snap={snap} intent={intent.value} goSources={() => setTab("sources")} />}
            {tab === "sources" && <Sources snap={snap} discover={discover} />}
            {tab === "wallpaper" && <Wallpaper snap={snap} />}
            {tab === "plate" && <Plate snap={snap} />}
            {tab === "rotation" && <Rotation snap={snap} go={(t) => setTab(asTab(t))} />}
            {tab === "library" && <Library snap={snap} />}
            {tab === "general" && <General snap={snap} />}
          </div>
        )}
      </main>
    </div>
  );
}
