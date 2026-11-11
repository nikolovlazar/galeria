import { useMemo, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, imageUrl } from "../shared/api";
import type { Display, FitMode, FrameStyle, Matte, Painting, Snapshot } from "../shared/types";
import { PlatePreview } from "./PlatePreview";
import { Button, Row, Section, Segmented, Select, Slider, Toggle, bytes, timeUntil, useLibrary } from "./ui";

type P = { snap: Snapshot };

// ---------------------------------------------------------------------------
// Now showing

export function NowShowing({ snap }: P) {
  if (!snap.library.root) {
    return (
      <div className="empty">
        <h2>Choose your art folder</h2>
        <p className="muted">Galeria reads a folder containing a <code>manifest.json</code> and your painting files. Nothing is copied or bundled.</p>
        <Button kind="primary" onClick={() => void chooseFolder()}>
          Choose Folder…
        </Button>
      </div>
    );
  }
  return (
    <>
      {snap.displays.map((d) => (
        <DisplayCard key={d.key} d={d} snap={snap} multi={snap.displays.length > 1} />
      ))}
    </>
  );
}

function DisplayCard({ d, snap, multi }: { d: Display; snap: Snapshot; multi: boolean }) {
  const p = d.painting;
  const next = timeUntil(d.nextChangeAt);
  return (
    <section className="section">
      <header className="section-head">
        <div>
          <h2>{multi ? d.name : "Now Showing"}</h2>
          <p className="muted">
            {d.width}×{d.height} pt · {(d.width / d.height).toFixed(2)}:1
            {next && ` · next painting ${next}`}
            {snap.settings.rotation.paused && " · rotation paused"}
          </p>
        </div>
        <div className="section-actions">
          <Button onClick={() => void api.previous(d.key)} disabled={d.historyLen === 0}>
            ← Previous
          </Button>
          <Button onClick={() => void api.next(d.key)}>Next →</Button>
        </div>
      </header>
      {p ? (
        <div className="now">
          <PlatePreview painting={p} plate={{ ...snap.settings.plate, defaultPosition: snap.settings.plate.positions[d.key] ?? snap.settings.plate.defaultPosition }} scale={0.42} className="now-art" />
          <PaintingDetails p={p} snap={snap} displayKey={d.key} />
        </div>
      ) : (
        <div className="card muted pad">{snap.library.error ?? "No painting available."}</div>
      )}
    </section>
  );
}

function PaintingDetails({ p, snap, displayKey, compact }: { p: Painting; snap: Snapshot; displayKey?: string; compact?: boolean }) {
  const multi = snap.displays.length > 1 && !snap.settings.wallpaper.sameOnAllDisplays;
  const rows: [string, string | null | undefined][] = [
    ["Artist", [p.artist, p.artistBio && `(${p.artistBio})`].filter(Boolean).join(" ")],
    ["Date", p.date],
    ["Medium", p.medium],
    ["Dimensions", p.dimensions],
    ["Culture", p.culture],
    ["Collection", [p.museum, p.creditLine].filter(Boolean).join(" · ")],
    ["Accession", p.accession],
    ["Image credit", p.credit],
    ["License", p.license],
    ["Scan", `${p.width}×${p.height} px · ${(p.width * p.height / 1e6).toFixed(1)} MP · ${bytes(p.bytes)}`],
    ["File", p.file],
  ];
  return (
    <div className="details">
      <h3>{p.title}</h3>
      {p.originalTitle && <div className="muted italic">{p.originalTitle}</div>}
      <dl>
        {rows
          .filter(([, v]) => v)
          .map(([k, v]) => (
            <div key={k}>
              <dt>{k}</dt>
              <dd>{v}</dd>
            </div>
          ))}
      </dl>
      {p.description && (
        <p className="description">
          {p.description}
          {p.descriptionSource && <span className="muted"> — {p.descriptionSource}{p.descriptionLang && p.descriptionLang !== "en" ? ` (${p.descriptionLang})` : ""}</span>}
        </p>
      )}
      {p.didYouKnow && !compact && (
        <p className="description">
          <strong>Did you know?</strong> {p.didYouKnow}
        </p>
      )}
      <div className="actions">
        {displayKey === undefined && (
          <Button kind="primary" onClick={() => void api.show(p.id)}>
            Show on Desktop
          </Button>
        )}
        {displayKey === undefined &&
          multi &&
          snap.displays.map((d) => (
            <Button key={d.key} onClick={() => void api.show(p.id, d.key)}>
              On {d.name}
            </Button>
          ))}
        <Button onClick={() => void api.openViewer(p.id, displayKey)}>Open Viewer</Button>
        {p.sourcePage && <Button onClick={() => void api.openUrl(p.sourcePage!)}>Museum Page</Button>}
        {p.wikipediaUrl && <Button onClick={() => void api.openUrl(p.wikipediaUrl!)}>Wikipedia</Button>}
        <Button onClick={() => void api.reveal(p.id)}>Show File</Button>
        <Button onClick={() => void api.setFavorite(p.id, !p.favorite)}>{p.favorite ? "★ Favorite" : "☆ Favorite"}</Button>
        <Button kind={p.excluded ? undefined : "danger"} onClick={() => void api.setExcluded(p.id, !p.excluded)}>
          {p.excluded ? "Allow Again" : "Never Show"}
        </Button>
        <Button kind="ghost" onClick={() => void api.refreshEnrichment(p.id)} title="Fetch description and artist details again">
          ↻ Refresh Info
        </Button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Browse

type Filter = "all" | "favorites" | "excluded" | "landscape" | "portrait" | "panorama";

export function Browse({ snap }: P) {
  const lib = useLibrary(snap);
  const [q, setQ] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [museum, setMuseum] = useState("");
  const [sort, setSort] = useState<"manifest" | "title" | "artist">("manifest");
  const [selected, setSelected] = useState<string | null>(null);
  const showing = new Set(snap.displays.map((d) => d.painting?.id));

  const museums = useMemo(() => [...new Set((lib?.paintings ?? []).map((p) => p.museum).filter(Boolean) as string[])].sort(), [lib]);
  const list = useMemo(() => {
    const needle = q.trim().toLowerCase();
    let ps = (lib?.paintings ?? []).filter((p) => {
      if (museum && p.museum !== museum) return false;
      if (filter === "favorites" && !p.favorite) return false;
      if (filter === "excluded" && !p.excluded) return false;
      if (filter === "landscape" && p.orientation !== "landscape") return false;
      if (filter === "portrait" && p.orientation !== "portrait") return false;
      if (filter === "panorama" && !(p.aspect > 2 || p.aspect < 0.5)) return false;
      if (!needle) return true;
      return [p.title, p.originalTitle, p.artist, p.date, p.museum, p.medium, ...p.tags].some((v) => v?.toLowerCase().includes(needle));
    });
    if (sort === "title") ps = [...ps].sort((a, b) => a.title.localeCompare(b.title));
    if (sort === "artist") ps = [...ps].sort((a, b) => (a.artist ?? "~").localeCompare(b.artist ?? "~"));
    return ps;
  }, [lib, q, filter, museum, sort]);
  const sel = lib?.paintings.find((p) => p.id === selected) ?? null;

  if (!lib) return <div className="muted pad">Loading library…</div>;
  return (
    <div className={`browse${sel ? " with-detail" : ""}`}>
      <div className="browse-main">
        <div className="toolbar">
          <input className="search" placeholder={`Search ${lib.paintings.length} paintings…`} value={q} onChange={(e) => setQ(e.target.value)} autoFocus />
          <Select
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: "All" },
              { value: "favorites", label: "Favorites" },
              { value: "excluded", label: "Hidden" },
              { value: "landscape", label: "Landscape" },
              { value: "portrait", label: "Portrait" },
              { value: "panorama", label: "Panoramas (pan)" },
            ]}
          />
          <Select value={museum} onChange={setMuseum} options={[{ value: "", label: "All museums" }, ...museums.map((m) => ({ value: m, label: m }))]} />
          <Select
            value={sort}
            onChange={setSort}
            options={[
              { value: "manifest", label: "Manifest order" },
              { value: "title", label: "Title" },
              { value: "artist", label: "Artist" },
            ]}
          />
          <span className="muted count">{list.length}</span>
        </div>
        <div className="grid">
          {list.map((p) => (
            <button key={p.id} type="button" className={`tile${p.excluded ? " excluded" : ""}${selected === p.id ? " selected" : ""}`} onClick={() => setSelected(p.id === selected ? null : p.id)} onDoubleClick={() => void api.openViewer(p.id)}>
              <div className="tile-img">
                <img src={imageUrl.thumb(p.id)} loading="lazy" decoding="async" alt="" />
                {showing.has(p.id) && <span className="badge">On desktop</span>}
                {p.favorite && <span className="star">★</span>}
              </div>
              <div className="tile-title">{p.title}</div>
              <div className="tile-sub">{[p.artist, p.date].filter(Boolean).join(", ")}</div>
            </button>
          ))}
        </div>
      </div>
      {sel && (
        <aside className="detail">
          <button type="button" className="close" onClick={() => setSelected(null)}>
            ×
          </button>
          <img className="detail-img" src={imageUrl.thumb(sel.id)} alt="" />
          <PaintingDetails p={sel} snap={snap} compact />
        </aside>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Wallpaper

const SPEEDS = [
  { value: 2, label: "Glacial" },
  { value: 5, label: "Very slow" },
  { value: 10, label: "Slow" },
  { value: 20, label: "Gentle" },
  { value: 40, label: "Brisk" },
];
const WALLS = ["#1d1c1a", "#46443f", "#5a1e22", "#1e2b3a", "#36402f", "#e6e3dc", "#000000"];

export function Wallpaper({ snap }: P) {
  const w = snap.settings.wallpaper;
  const set = (patch: Partial<typeof w>) => void api.patch({ wallpaper: patch });
  const d = snap.displays[0];
  const panExample = d ? (d.width * 2.9) - d.width : 0;
  const sweepMin = panExample / w.panSpeed / 60;
  return (
    <>
      <Section title="Desktop" description="How paintings meet your screen.">
        <Row label="Live desktop" hint={snap.liveSupported ? "Renders the painting, its info plate and slow panning behind your desktop icons." : "Not available on this desktop session (Wayland) — Galeria sets the system wallpaper instead."} disabled={!snap.liveSupported}>
          <Toggle checked={w.live && snap.liveSupported} onChange={(v) => set({ live: v })} disabled={!snap.liveSupported} />
        </Row>
        <Row label="Also set the system wallpaper" hint="Keeps Mission Control, Spaces and the lock screen in sync. Your original wallpaper is remembered.">
          <Toggle checked={w.syncSystem} onChange={(v) => set({ syncSystem: v })} />
        </Row>
        {snap.originalWallpapers > 0 && (
          <Row label="Original wallpaper" hint="Turns off syncing and puts your previous wallpaper back.">
            <Button onClick={() => void api.restoreOriginalWallpaper()}>Restore</Button>
          </Row>
        )}
      </Section>

      <Section title="Fit" description="Galeria compares each painting's aspect ratio with the display's.">
        <Row label="Mode">
          <Segmented<FitMode>
            value={w.fit}
            onChange={(v) => set({ fit: v })}
            options={[
              { value: "auto", label: "Automatic" },
              { value: "fit", label: "Entire painting" },
              { value: "fill", label: "Fill screen" },
              { value: "pan", label: "Fill & pan" },
            ]}
          />
        </Row>
        {w.fit === "auto" && (
          <Row label="Fill when the crop is under" hint="Paintings close to the screen's shape fill it edge to edge; anything more unusual pans (or hangs on the wall).">
            <Slider value={w.cropTolerance} min={0} max={0.3} step={0.01} onCommit={(v) => set({ cropTolerance: v })} format={(v) => `${Math.round(v * 100)}%`} />
          </Row>
        )}
        <Row label="Pan wide & tall paintings" hint="Fill the screen and drift edge to edge so you see the whole work over time.">
          <Toggle checked={w.pan} onChange={(v) => set({ pan: v })} />
        </Row>
        <Row label="Panning speed" hint={`A 2.9:1 panorama on this display takes about ${sweepMin < 1 ? "<1" : Math.round(sweepMin)} min per sweep.`}>
          <Segmented<number> value={SPEEDS.some((s) => s.value === w.panSpeed) ? w.panSpeed : -1} onChange={(v) => set({ panSpeed: v })} options={SPEEDS} />
        </Row>
        <Row label="Custom speed">
          <Slider value={w.panSpeed} min={0.5} max={60} step={0.5} onCommit={(v) => set({ panSpeed: v })} format={(v) => `${v} px/s`} />
        </Row>
        <Row label="Rest at each edge">
          <Slider value={w.panPause} min={0} max={180} step={5} onCommit={(v) => set({ panPause: v })} format={(v) => `${v} s`} />
        </Row>
        <Row label="Pause panning">
          <Toggle checked={w.panPaused} onChange={(v) => set({ panPaused: v })} />
        </Row>
      </Section>

      <Section title="Wall" description="What surrounds a painting that's shown in full.">
        <Row label="Wall">
          <Segmented<Matte>
            value={w.matte}
            onChange={(v) => set({ matte: v })}
            options={[
              { value: "average", label: "Tinted to painting" },
              { value: "blur", label: "Blurred painting" },
              { value: "color", label: "Colour" },
            ]}
          />
        </Row>
        <Row label="Wall colour" disabled={w.matte !== "color"}>
          <div className="swatches">
            {WALLS.map((c) => (
              <button key={c} type="button" className={`swatch${w.matteColor === c && w.matte === "color" ? " active" : ""}`} style={{ background: c }} onClick={() => set({ matte: "color", matteColor: c })} />
            ))}
            <input type="color" value={w.matteColor} onChange={(e) => set({ matte: "color", matteColor: e.target.value })} />
          </div>
        </Row>
        <Row label="Frame">
          <Segmented<FrameStyle>
            value={w.frame}
            onChange={(v) => set({ frame: v })}
            options={[
              { value: "none", label: "None" },
              { value: "black", label: "Black" },
              { value: "white", label: "White" },
              { value: "gilded", label: "Gilded" },
              { value: "walnut", label: "Walnut" },
            ]}
          />
        </Row>
        <Row label="Space around the painting">
          <Slider value={w.fitMargin} min={0} max={0.2} step={0.01} onCommit={(v) => set({ fitMargin: v })} format={(v) => `${Math.round(v * 100)}%`} />
        </Row>
        <Row label="Shadow on the wall">
          <Toggle checked={w.shadow} onChange={(v) => set({ shadow: v })} />
        </Row>
        <Row label="Gallery spotlight">
          <Toggle checked={w.spotlight} onChange={(v) => set({ spotlight: v })} />
        </Row>
        <Row label="Crossfade">
          <Slider value={w.transitionSeconds} min={0} max={6} step={0.25} onCommit={(v) => set({ transitionSeconds: v })} format={(v) => `${v} s`} />
        </Row>
      </Section>

      <Section title="Displays">
        <Row label="Same painting on every display">
          <Toggle checked={w.sameOnAllDisplays} onChange={(v) => set({ sameOnAllDisplays: v })} />
        </Row>
        <Row label="Match display orientation" hint="Only landscape works on landscape screens and portrait works on portrait screens.">
          <Toggle checked={w.matchOrientation} onChange={(v) => set({ matchOrientation: v })} />
        </Row>
        {snap.displays.map((d) => (
          <Row key={d.key} label={d.name} hint={`${d.width}×${d.height} pt @${d.scale}x${d.primary ? " · main display" : ""}`} />
        ))}
      </Section>
    </>
  );
}

// ---------------------------------------------------------------------------
// Rotation

export function Rotation({ snap }: P) {
  const r = snap.settings.rotation;
  const set = (patch: Partial<typeof r>) => void api.patch({ rotation: patch });
  return (
    <Section title="Rotation" description="When and in what order paintings change.">
      <Row label="Change painting">
        <Select
          value={r.intervalMinutes}
          onChange={(v) => set({ intervalMinutes: v })}
          options={[
            { value: 0, label: "Never" },
            { value: 1, label: "Every minute" },
            { value: 5, label: "Every 5 minutes" },
            { value: 15, label: "Every 15 minutes" },
            { value: 30, label: "Every 30 minutes" },
            { value: 60, label: "Every hour" },
            { value: 180, label: "Every 3 hours" },
            { value: 720, label: "Every 12 hours" },
            { value: 1440, label: "Every day" },
          ]}
        />
      </Row>
      <Row label="Order" hint="Shuffle goes through the whole library before repeating.">
        <Segmented
          value={r.order}
          onChange={(v) => set({ order: v })}
          options={[
            { value: "shuffle", label: "Shuffle" },
            { value: "sequential", label: "Manifest order" },
          ]}
        />
      </Row>
      <Row label="Pause rotation">
        <Toggle checked={r.paused} onChange={(v) => set({ paused: v })} />
      </Row>
      <Row label="Change now">
        <Button onClick={() => void api.next()}>Next Painting</Button>
      </Row>
    </Section>
  );
}

// ---------------------------------------------------------------------------
// Info plate

const POSITIONS: [number, number, string][] = [
  [0, 0, "↖"],
  [0.5, 0, "↑"],
  [1, 0, "↗"],
  [0, 0.5, "←"],
  [0.5, 0.5, "•"],
  [1, 0.5, "→"],
  [0, 1, "↙"],
  [0.5, 1, "↓"],
  [1, 1, "↘"],
];

export function Plate({ snap }: P) {
  const ps = snap.settings.plate;
  const set = (patch: Record<string, unknown>) => void api.patch({ plate: patch });
  const [newName, setNewName] = useState("");
  const sample = snap.displays.find((d) => d.painting)?.painting ?? null;
  const styles = [...snap.builtinStyles.map((s) => ({ ...s, custom: false })), ...snap.customStyles.map((s) => ({ id: s.id, name: s.name, custom: true }))];
  const fields: [keyof typeof ps.fields, string][] = [
    ["artist", "Artist"],
    ["artistBio", "Nationality & life dates"],
    ["date", "Date"],
    ["medium", "Medium"],
    ["dimensions", "Dimensions"],
    ["description", "Description"],
    ["museum", "Museum & credit line"],
    ["credit", "Image credit"],
    ["license", "License"],
    ["accession", "Accession number"],
    ["qr", "QR code to the museum page"],
  ];
  return (
    <>
      <Section
        title="Style"
        description="Every style renders live over the current painting."
        actions={
          <>
            <Button onClick={() => void api.openStylesFolder()}>Open Styles Folder</Button>
          </>
        }
      >
        <div className="styles-grid">
          {styles.map((s) => (
            <div key={s.id} role="button" tabIndex={0} className={`style-card${ps.style === s.id ? " active" : ""}`} onClick={() => set({ style: s.id, visible: true })}>
              {sample ? <PlatePreview painting={sample} plate={{ ...ps, style: s.id, defaultPosition: { x: 0.5, y: 1 } }} scale={0.5} /> : <div className="plate-preview" />}
              <div className="style-name">
                {s.name}
                {s.custom && (
                  <button
                    type="button"
                    className="link"
                    onClick={(e) => {
                      e.stopPropagation();
                      void api.openStyleFile(s.id);
                    }}
                  >
                    Edit CSS
                  </button>
                )}
              </div>
            </div>
          ))}
        </div>
        <div className="row">
          <div className="row-label">
            <div>New custom style</div>
            <div className="row-hint">Creates a documented CSS file in the styles folder and opens it. Saving the file updates the desktop instantly.</div>
          </div>
          <div className="row-control inline">
            <input className="text" placeholder="Style name" value={newName} onChange={(e) => setNewName(e.target.value)} />
            <Button
              disabled={!newName.trim()}
              onClick={() => {
                void api.createCustomStyle(newName.trim());
                setNewName("");
              }}
            >
              Create
            </Button>
          </div>
        </div>
      </Section>

      <Section title="Plate">
        <Row label="Show the info plate">
          <Toggle checked={ps.visible} onChange={(v) => set({ visible: v })} />
        </Row>
        <Row label="Size">
          <Slider value={ps.scale} min={0.6} max={1.8} step={0.05} onCommit={(v) => set({ scale: v })} format={(v) => `${Math.round(v * 100)}%`} />
        </Row>
        <Row label="Width">
          <Slider value={ps.width} min={240} max={640} step={10} onCommit={(v) => set({ width: v })} format={(v) => `${v} pt`} />
        </Row>
        <Row label="Description length">
          <Select
            value={ps.descriptionLines}
            onChange={(v) => set({ descriptionLines: v })}
            options={[
              { value: 3, label: "3 lines" },
              { value: 4, label: "4 lines" },
              { value: 6, label: "6 lines" },
              { value: 8, label: "8 lines" },
              { value: 12, label: "12 lines" },
              { value: 0, label: "Everything" },
            ]}
          />
        </Row>
        <Row label="Descriptions in other languages" hint="Some museum texts only exist in the original language (the Rijksmuseum often writes in Dutch).">
          <Toggle checked={ps.foreignDescriptions} onChange={(v) => set({ foreignDescriptions: v })} />
        </Row>
        <Row label="Fade out after" hint="The plate reappears with every new painting.">
          <Select
            value={ps.autoHideSeconds}
            onChange={(v) => set({ autoHideSeconds: v })}
            options={[
              { value: 0, label: "Never" },
              { value: 10, label: "10 seconds" },
              { value: 30, label: "30 seconds" },
              { value: 60, label: "1 minute" },
              { value: 300, label: "5 minutes" },
            ]}
          />
        </Row>
      </Section>

      <Section title="Position" description="Pick a spot, or drag the plate directly on the desktop.">
        <Row label="Anchor" hint={Object.keys(ps.positions).length ? "Some displays use a custom dragged position; picking an anchor resets them." : undefined}>
          <div className="position-grid">
            {POSITIONS.map(([x, y, icon]) => {
              const on = !Object.keys(ps.positions).length && ps.defaultPosition.x === x && ps.defaultPosition.y === y;
              return (
                <button key={`${x},${y}`} type="button" className={on ? "active" : ""} onClick={() => void api.setPlatePosition("wallpaper", null, x, y)}>
                  {icon}
                </button>
              );
            })}
          </div>
        </Row>
        <Row label="Drag on the desktop" hint="Lifts the wallpaper above your icons until you press Done (or Esc)." disabled={!(snap.settings.wallpaper.live && snap.liveSupported)}>
          <Button onClick={() => void api.setArrange(!snap.arrange)} kind={snap.arrange ? "primary" : undefined} disabled={!(snap.settings.wallpaper.live && snap.liveSupported)}>
            {snap.arrange ? "Done Arranging" : "Arrange…"}
          </Button>
        </Row>
        <Row label="Distance from screen edges">
          <Slider value={ps.margin} min={0} max={160} step={4} onCommit={(v) => set({ margin: v })} format={(v) => `${v} pt`} />
        </Row>
      </Section>

      <Section title="Details">
        <div className="checks">
          {fields.map(([k, label]) => (
            <label key={k} className="check">
              <input type="checkbox" checked={ps.fields[k]} onChange={(e) => set({ fields: { [k]: e.target.checked } })} />
              {label}
            </label>
          ))}
        </div>
      </Section>
    </>
  );
}

// ---------------------------------------------------------------------------
// Library

async function chooseFolder() {
  const dir = await openDialog({ directory: true, multiple: false, title: "Choose the folder that contains manifest.json" });
  if (typeof dir === "string") await api.setLibraryPath(dir);
}

export function Library({ snap }: P) {
  const lib = useLibrary(snap);
  const l = snap.library;
  const [cleared, setCleared] = useState<string | null>(null);
  return (
    <>
      <Section title="Art Folder" description="Galeria never bundles or copies paintings — it reads them from this folder.">
        <Row label={l.root ?? "No folder chosen"} hint={l.generated ? `Manifest generated ${l.generated}` : undefined}>
          <div className="inline">
            <Button onClick={() => void chooseFolder()}>{l.root ? "Change…" : "Choose…"}</Button>
            {l.root && <Button onClick={() => void api.rescan()}>Rescan</Button>}
          </div>
        </Row>
        {l.error && <div className="error-row">{l.error}</div>}
        <div className="stats">
          <Stat n={l.entries} label="manifest entries" />
          <Stat n={l.valid} label="ready" ok />
          <Stat n={l.missing} label="missing files" warn={l.missing > 0} />
          <Stat n={l.invalid} label="invalid" warn={l.invalid > 0} />
          <Stat n={l.duplicates} label="duplicates" warn={l.duplicates > 0} />
          <Stat n={l.unindexed} label="files not in manifest" />
          <Stat n={l.excluded} label="hidden by you" />
        </div>
        <p className="muted small pad-x">The folder is watched: when the manifest changes or files finish downloading, the library updates by itself.</p>
      </Section>

      {lib && (lib.missing.length > 0 || lib.invalid.length > 0 || lib.duplicates.length > 0) && (
        <Section title="Issues">
          <ul className="issues">
            {[...lib.missing, ...lib.invalid, ...lib.duplicates].map((i, n) => (
              <li key={n}>
                <code>{i.file}</code> <span className="muted">— {i.problem}</span>
              </li>
            ))}
          </ul>
        </Section>
      )}
      {lib && lib.unindexed.length > 0 && (
        <Section title="Files Not in the Manifest" description="These images are in the folder but won't be shown until the manifest lists them.">
          <ul className="issues">
            {lib.unindexed.map((f) => (
              <li key={f}>
                <code>{f}</code>
              </li>
            ))}
            {l.unindexed > lib.unindexed.length && <li className="muted">…and {l.unindexed - lib.unindexed.length} more</li>}
          </ul>
        </Section>
      )}

      <Section title="Museum Information" description="Fills in descriptions, artist nationality & dates, credit lines and English titles from the Cleveland Museum of Art and Rijksmuseum APIs, Wikidata and Wikipedia. Cached locally; your manifest is never modified.">
        <Row label="Look up information online">
          <Toggle checked={snap.settings.general.enrichOnline} onChange={(v) => void api.patch({ general: { enrichOnline: v } })} />
        </Row>
        <Row label="Progress" hint={l.enrichBusy ? "Working through the library in the background…" : undefined}>
          <span className="muted">
            {l.enriched} of {l.valid} enriched
          </span>
        </Row>
        <Row label="Start over" hint="Forget everything fetched and look it up again.">
          <Button onClick={() => void api.refreshEnrichment()}>Refresh All</Button>
        </Row>
      </Section>

      <Section title="Cache">
        <Row label="Thumbnails & display-sized renditions" hint={cleared ?? "Rebuilt automatically when needed."}>
          <Button
            onClick={async () => {
              const n = await api.clearImageCache();
              setCleared(`Freed ${bytes(n)}`);
            }}
          >
            Clear Cache
          </Button>
        </Row>
      </Section>

      <Section title="Manifest Format">
        <div className="pad">
          <p className="muted small">
            <code>manifest.json</code> is an array (or <code>{"{ \"items\": [...] }"}</code>) of entries. Only <code>file</code> is required; everything else appears on the plate when present.
          </p>
          <pre className="code">{`{
  "items": [{
    "file": "van-gogh__wheat-field.jpg",      // relative to this folder
    "title": "Wheat Field with Cypresses",
    "artist": "Vincent van Gogh",
    "artist_bio": "Dutch, 1853–1890",
    "date": "1889",
    "medium": "oil on canvas",
    "dimensions": "73 × 93.4 cm",
    "description": "Painted in Saint-Rémy…",
    "museum": "The Metropolitan Museum of Art",
    "credit_line": "Purchase, The Annenberg Foundation Gift, 1993",
    "credit": "Image: The Met (CC0)",
    "license": "CC0",
    "source": "met", "source_id": "436535",
    "source_page": "https://www.metmuseum.org/art/collection/search/436535",
    "tags": ["landscape"]
  }]
}`}</pre>
        </div>
      </Section>
    </>
  );
}

function Stat({ n, label, warn, ok }: { n: number; label: string; warn?: boolean; ok?: boolean }) {
  return (
    <div className={`stat${warn ? " warn" : ""}${ok ? " ok" : ""}`}>
      <div className="stat-n">{n}</div>
      <div className="stat-label">{label}</div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// General

export function General({ snap }: P) {
  const g = snap.settings.general;
  const v = snap.settings.viewer;
  const setG = (patch: Partial<typeof g>) => void api.patch({ general: patch });
  const setV = (patch: Partial<typeof v>) => void api.patch({ viewer: patch });
  return (
    <>
      <Section title="App">
        {snap.platform === "macos" && (
          <Row label="Show in Dock" hint="Galeria always lives in the menu bar.">
            <Toggle checked={g.showInDock} onChange={(x) => setG({ showInDock: x })} />
          </Row>
        )}
        <Row label="Launch at login">
          <Toggle checked={g.launchAtLogin} onChange={(x) => setG({ launchAtLogin: x })} />
        </Row>
        <Row label="Restore my original wallpaper when quitting">
          <Toggle checked={g.restoreWallpaperOnQuit} onChange={(x) => setG({ restoreWallpaperOnQuit: x })} />
        </Row>
      </Section>
      <Section title="Viewer" description="Full-screen, distraction-free viewing. Move the mouse to reveal the painting edge to edge.">
        <Row label="Open the viewer">
          <Button onClick={() => void api.openViewer()}>Open Viewer</Button>
        </Row>
        <Row label="Motion smoothing">
          <Slider value={v.smoothing} min={0} max={0.95} step={0.05} onCommit={(x) => setV({ smoothing: x })} format={(x) => (x < 0.2 ? "Direct" : x < 0.7 ? "Smooth" : "Floaty")} />
        </Row>
        <Row label="Edge dead zone" hint="How close to the screen edge the mouse must go to reach the painting's edge.">
          <Slider value={v.edgeDeadZone} min={0} max={0.3} step={0.01} onCommit={(x) => setV({ edgeDeadZone: x })} format={(x) => `${Math.round(x * 100)}%`} />
        </Row>
        <Row label="Hide the cursor over the painting">
          <Toggle checked={v.hideCursor} onChange={(x) => setV({ hideCursor: x })} />
        </Row>
        <Row label="Show a minimap while exploring">
          <Toggle checked={v.minimap} onChange={(x) => setV({ minimap: x })} />
        </Row>
        <Row label="Show the info plate">
          <Toggle checked={v.showPlate} onChange={(x) => setV({ showPlate: x })} />
        </Row>
      </Section>
      <Section title="About">
        <div className="pad about">
          <img src="/galeria.png" alt="" width={56} height={56} />
          <div>
            <strong>Galeria</strong> {snap.version}
            <div className="muted small">
              <em>galeria</em> — Medieval Latin for a long portico or gallery. Paintings stay in your own folder; Galeria only reads them.
            </div>
          </div>
        </div>
        <Row label="Quit Galeria">
          <Button kind="danger" onClick={() => void api.quit()}>
            Quit
          </Button>
        </Row>
      </Section>
    </>
  );
}
