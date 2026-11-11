# Galeria

*Galeria* is Medieval Latin for a gallery. (The classical word for a picture gallery is *pinacotheca*.)

Galeria turns a folder of high-resolution painting scans into a museum on your desktop:

- **Live desktop.** The painting is drawn in a window that sits behind your desktop icons, the same technique [Plash](https://github.com/sindresorhus/Plash) uses. It can also set the real system wallpaper, so Mission Control, Spaces and the lock screen match.
- **Fitted to every display.** Galeria compares each painting's aspect ratio with the display's. Works close to the screen's shape fill it edge to edge. Unusually wide or tall works slowly pan from one edge to the other so you see the whole canvas over time, or they hang on a gallery wall (tinted to the painting, blurred, or a colour of your choice) with an optional gilded, walnut, black or white frame.
- **Museum info plate.** Title, artist, nationality and life dates, date, medium, dimensions, description, collection and credit line, accession number, image credit, license, and an optional QR code to the museum page. You can hide the plate, snap it to a corner, or drag it anywhere on the desktop.
- **Plate styles.** Museum Plastic, Liquid Glass, Smoked Glass, Brass Plaque, Paper Card, Black Acrylic and Minimal are built in. You can add your own as plain CSS files, and they hot-reload.
- **Full-screen viewer.** A distraction-free mode where the mouse reveals the painting: move left/right (or up/down) to travel edge to edge, scroll or pinch to zoom up to past 1:1, and use the minimap to see where you are.
- **Menu bar app.** Every option lives in the menu bar: painting, rotation, fit, panning, wall, frame and plate. There's no Dock icon unless you turn it on.
- **Nothing bundled.** Galeria stores only the location of your art folder. It reads `manifest.json` from that folder, checks that every file it lists exists and is a readable image, and watches the folder for changes. The paintings are never copied or shipped with the app.
- **Optional museum lookup.** Galeria can fill in missing descriptions, artist details, credit lines and English titles from the Cleveland Museum of Art and Rijksmuseum open APIs, Wikidata and Wikipedia. Results are cached locally, not written to the manifest.
- **Discover and download, legally.** Browse thumbnails from the National Gallery of Art, the Cleveland Museum of Art, the Rijksmuseum, The Met, the Art Institute of Chicago and Wikimedia Commons, pick the paintings you like and download them into your art folder. Every download is checked against licensing rules compiled into the app. See [Downloading paintings](#downloading-paintings).
- **Show everything, or a group.** Rotate through the whole library, your favorites, a playlist, chosen museums and genres, or a hand-picked selection. Museums can be switched off without deleting anything.
- **Light and dark.** Galeria measures how bright each painting is and can show brighter paintings in Light Mode and darker ones in Dark Mode.

![Desktop: Degas, *Before the Ballet*, hung in a gilded frame on a wall tinted to the painting](docs/desktop.jpg)
![Viewer: moving the mouse reveals a 2.6:1 panorama edge to edge](docs/viewer.jpg)
![Settings: every plate style rendered live](docs/settings.jpg)
![Discover: Rijksmuseum results, with the license decision for the selected painting](docs/discover.jpg)

## Building

Requirements: Rust (the pinned toolchain in `rust-toolchain.toml` installs itself via rustup), Node 20+, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```sh
npm install
npm run tauri dev        # run in development
npm run app              # build Galeria.app (macOS) → src-tauri/target/release/bundle/macos/
npm run tauri build      # installers for the current platform (.dmg / .msi / .deb / .AppImage …)
```

Run the Rust tests with `cd src-tauri && cargo test`.

On first launch Galeria asks for your art folder. You can also set it up front with `GALERIA_LIBRARY=/path/to/folder`.

## Using it

### Menu bar

The menu bar item has everything: next/previous painting, *Open in Viewer*, the museum page / Wikipedia / file for the current painting, *Never Show Again*, the rotation interval and order, fit mode, panning (on/off, pause, speed), the wall and frame, the info plate (show/hide, style, position, size, which details appear, *Drag on Desktop…*), library status and issues, and options for the system wallpaper, Dock icon, launch at login and online lookups.

### Moving the plate

Choose **Info Plate → Position → Drag on Desktop…**. This lifts the wallpaper above the desktop icons, though still below every app window. Drag the plate anywhere; it snaps to edges and centre lines. Press **Done** or **Esc** when you're finished. Each display remembers its own position.

### Viewer

| Key | Action |
| --- | --- |
| move the mouse | reveal the painting edge to edge |
| scroll / pinch | zoom |
| double-click, `F` | whole painting ↔ fill the screen |
| `0` | reset zoom |
| `←` `→` | previous / next painting |
| `I` | show or hide the plate (drag it anywhere) |
| `W` | use as desktop wallpaper |
| `O` | open the museum page |
| `?` | help |
| `Esc` | close |

### Command line

When Galeria is already running, launching it again with a flag controls the running app. This makes it easy to bind hotkeys with Raycast, Alfred, skhd, Hammerspoon or similar:

```sh
/Applications/Galeria.app/Contents/MacOS/galeria --next
```

| Flag | |
| --- | --- |
| `--next`, `--prev` | change the painting on every display |
| `--show <id>` | show one painting now, by its manifest id (e.g. `nga:100922`) |
| `--viewer` | open the full-screen viewer |
| `--settings [section]` | open settings (`now`, `browse`, `discover`, `sources`, `rotation`, `wallpaper`, `plate`, `library`, `general`) |
| `--toggle-plate`, `--toggle-panning` | |
| `--arrange`, `--done` | enter / leave plate-arranging mode |
| `--menu <id>` | trigger any menu item, e.g. `--menu style:glass`, `--menu fit:pan`, `--menu speed:10`, `--menu interval:60`, `--menu scope:favorites`, `--menu scope-genre:seascape`, `--menu match-appearance` |
| `--theme light\|dark\|auto` | pretend the system is in that appearance (to try light/dark matching) |
| `--capture <dir>` | write PNG snapshots of every Galeria window (macOS, debugging) |
| `--ui <action>` | drive the settings window without a mouse: `click:<css selector>`, `type:<selector>=<text>`, `key:<selector>=Enter` (testing) |
| `--debug` | every window logs its render state, and the core logs what is showing and why |

Set `GALERIA_HOME=/some/folder` to keep settings, state and caches in a folder of your choosing instead of the system locations. It is handy for trying things against a copy of your library.

## Choosing what is shown

**Rotation → What to Show** (or **Show** in the menu bar) decides which paintings take part:

| Choice | Paintings |
| --- | --- |
| Everything | the whole library |
| Favorites | paintings you starred |
| A playlist | a named set. Create playlists in Rotation; fill them in Browse with **Select → Add to playlist** |
| Chosen museums and genres | any mix, e.g. seascapes from the Rijksmuseum |
| My selection | a one-off set: in Browse, **Select**, tick paintings, **Show Only These** |

Two things always apply on top. Paintings you marked *Never Show* stay hidden, and so do museums switched off in the **Museums** tab (nothing is deleted; switch them back on any time). If a choice leaves nothing to show, Galeria shows everything rather than a blank desktop, and says so.

**Genres** come from the manifest's `tags`, `kind` and `category`, plus keywords in the title. The list (Landscape, Seascape & Marine, Rivers & Lakes, Cityscape & Architecture, Winter, Night & Sunset, Gardens & Flowers, Still Life, Figures & Everyday Life, Portrait, Religious, Orthodox & Icons, Mythology & History, Animals, East Asian) lives in [`src-tauri/resources/genres.json`](src-tauri/resources/genres.json).

### Light and dark

Turn on **Rotation → Light & Dark → Match the system appearance** and Galeria shows brighter paintings in Light Mode and darker ones in Dark Mode. When the appearance switches, a painting that no longer fits is replaced straight away (optional).

Brightness is the mean luma (Rec. 601, 0–255) of the painting. Galeria uses the manifest's `brightness` when it is there and measures the rest itself in the background. The dividing line is automatic by default: the median of your library, so each mode gets about half of it. You can set your own line instead, and override any single painting to Light, Dark or Both in Browse.

## Downloading paintings

The **Discover** tab searches a museum, shows thumbnails with the image size and license, and downloads what you select into your art folder. Downloads are saved as JPEG next to your other paintings (larger originals are scaled to a long edge you choose, 8,000 px by default), measured for brightness, and added to `manifest.json`, so they join the rotation at once.

### The licensing rules

Galeria downloads an image only when its rights holder has put it in the public domain or under a license that lets anyone use it freely: **CC0, Public Domain Mark, CC BY or CC BY-SA**. NonCommercial and NoDerivatives licenses, "in copyright" statements, and anything it does not recognise are refused.

The rules are data, in [`src-tauri/resources/licensing.json`](src-tauri/resources/licensing.json), compiled into the app and shown in the **Museums** tab. For each source they name the field of the museum's own record that proves a work is open:

| Source | Programme | Proof Galeria checks on every record |
| --- | --- | --- |
| National Gallery of Art | Open Access, CC0 | `openaccess = 1` in the Gallery's open dataset |
| Cleveland Museum of Art | Open Access, CC0 | `share_license_status = CC0` |
| Rijksmuseum | public domain / CC0 | the record's rights statement is Public Domain Mark or CC0, and the image is marked downloadable |
| The Metropolitan Museum of Art | Open Access, CC0 | `isPublicDomain = true` |
| Art Institute of Chicago | Open Access, CC0 | `is_public_domain = true` |
| Wikimedia Commons | per file | the file's own license, matched against the accepted list |

A download never trusts what the settings window sends. The Rust side reads the record again from the museum, evaluates the rule again, and only then fetches the image. The license, its URL, the evidence (for example `share_license_status = CC0`) and the date of the check are written into the manifest entry.

Museums without an open-access programme (the Louvre, the Musée d'Orsay, the Prado, the Uffizi, the Kunsthistorisches Museum, the Tretyakov Gallery and others) are listed in the same file with what their terms say. Galeria never downloads from them. Their works are offered only as Wikimedia Commons files whose own license passes. Photographs under CC BY or CC BY-SA require credit, so for those the plate always shows the photographer and license, even when the credit line is switched off.

The rules were reviewed on the date in the file. Museums say they release only works they believe to be in the public domain and give no warranty about third-party rights; neither does Galeria. This is not legal advice.

### What Galeria changes in your folder

Galeria writes to the art folder only when you download something, or when you ask it to remove manifest entries whose files are gone. It adds the image file and appends one entry to `manifest.json`. Existing entries, key order and indentation are left exactly as they were. The file is re-read just before writing and replaced atomically, and a copy of the previous manifest is kept in the app data folder (`manifest-backups/`, the last eight). If another tool later rewrites the manifest without Galeria's entries, Galeria lists its downloads again as long as the files are still there.

The National Gallery of Art has no search API. The first time you open it in Discover, Galeria downloads the Gallery's open dataset once, keeps a small index of the open-access paintings in the cache folder and searches that locally.

## The art folder

The folder must contain `manifest.json` next to the images. The manifest can be a bare array, `{ "items": [...] }`, `{ "paintings": [...] }` or `{ "artworks": [...] }`. Only `file` is required; it must be a path relative to the folder, and subfolders are fine.

```jsonc
{
  "items": [
    {
      "file": "van-gogh__wheat-field-with-cypresses.jpg",
      "id": "met:436535",                     // optional stable id (defaults to `file`)
      "title": "Wheat Field with Cypresses",
      "artist": "Vincent van Gogh",
      "artist_bio": "Dutch, 1853–1890",
      "date": "1889",
      "medium": "oil on canvas",
      "dimensions": "73 × 93.4 cm",
      "description": "Painted in Saint-Rémy…",
      "museum": "The Metropolitan Museum of Art",
      "credit_line": "Purchase, The Annenberg Foundation Gift, 1993",
      "accession": "1993.132",
      "credit": "Image: The Met (CC0)",
      "license": "CC0",
      "source": "met",                        // nga | cma | rijks | met | … (used for online lookups)
      "source_id": "436535",
      "source_page": "https://www.metmuseum.org/art/collection/search/436535",
      "tags": ["landscape"],
      "width": 4000, "height": 3184           // optional; read from the file when missing
    }
  ]
}
```

Common aliases work too (`path`, `name`, `painter`, `year`, `period`, `creditLine`, `url`, …). Any other scalar fields are kept and passed to the plate. Optional `brightness` (0–255), `kind`, `category` and `attribution_required` are used for light/dark matching, genres and the credit line.

Validation runs on every change. The Library tab and the menu show which entries point to missing files, which are invalid (not an image, path escapes the folder, missing `file`), which are duplicates, and which images in the folder aren't in the manifest. Paintings whose files are missing are skipped, as are files modified in the last 20 seconds, so a half-downloaded image never becomes your wallpaper.

## Custom plate styles

**Info Plate → New Custom Style** creates a documented CSS file in the styles folder and opens it. On macOS that folder is `~/Library/Application Support/com.nikolovlazar.galeria/plate-styles/`. Saving the file updates every display immediately. The file name is the style id:

```css
/* name: Neon Night */
.plate[data-style="neon-night"] { background: rgba(8, 10, 24, .78); color: #e8fbff; border-radius: .8em; }
.plate[data-style="neon-night"] .plate-title { color: #ff5ec8; }
```

Available elements: `.plate-body`, `.plate-heading` (`.plate-title`, `.plate-date`), `.plate-original-title`, `.plate-artist`, `.plate-bio`, `.plate-meta` (`.plate-medium`, `.plate-dimensions`), `.plate-description`, `.plate-footer` (`.plate-museum`, `.plate-credit-line`, `.plate-accession`, `.plate-credit`, `.plate-license`, `.plate-text-credit`) and `.plate-qr`. Use `em` units so the size setting scales your style. See [`examples/plate-styles/`](examples/plate-styles/).

## Online lookups

When enabled (the default), a background worker goes through the library about once a second:

| Source | What it adds |
| --- | --- |
| Cleveland Museum of Art open-access API | description, "did you know", artist nationality & dates, credit line, accession number, technique, measurements |
| Rijksmuseum Linked Art | English title, description (often only in Dutch; hidden unless *Descriptions in other languages* is on), dimensions, object number |
| Wikidata (via the museum's own object ID) | artist nationality and life dates, the Wikipedia article |
| Wikipedia | a description when the museum has none (credited as CC BY-SA on the plate) |

The cache lives in `enrichment.json` in the app data folder. Values from your manifest always take precedence.

## Platform notes

| | Live desktop | System wallpaper | Status |
| --- | --- | --- | --- |
| **macOS** | NSWindow at `kCGDesktopWindowLevel`, on all Spaces | `NSWorkspace.setDesktopImageURL` (per display, fit/fill + fill colour) | daily driver |
| **Windows** | WebView parented to the desktop `WorkerW` (behind the icons) | `IDesktopWallpaper` (per monitor) | compiles; not yet tested on hardware |
| **Linux / X11** | `_NET_WM_WINDOW_TYPE_DESKTOP`, kept below and sticky | GNOME, Cinnamon, MATE, KDE, XFCE, sway, Hyprland (`swww`/`hyprpaper`), or `feh` | compiles; not yet tested on hardware |
| **Linux / Wayland** | not possible without layer-shell, so live mode turns off | as above | |

On macOS, `NSWorkspace` only changes the wallpaper of the current Space. The live desktop window shows on every Space regardless.

## Where things are stored

- Settings: `settings.json` in the app config dir (`~/Library/Application Support/com.nikolovlazar.galeria/` on macOS)
- Runtime state (what's showing, history, your original wallpaper): `state.json`
- Online lookup cache: `enrichment.json`
- Brightness Galeria measured itself: `tones.json`
- What Galeria downloaded: `downloads.json`; copies of the manifest from before each change: `manifest-backups/`
- The National Gallery of Art index: `catalogs/` in the app cache dir
- Thumbnails and display-sized renditions: the app cache dir. Clear them from *Library → Cache*.

## How it's built

[Tauri 2](https://v2.tauri.app). The backend is Rust and the windows are system webviews (WebKit on macOS and Linux, WebView2 on Windows).

```
src-tauri/src/
  core.rs       state, rotation, assignments, system-wallpaper sync, enrichment worker
  library.rs    manifest parsing + validation
  images.rs     galeria:// protocol: originals, display-sized renditions, thumbnails (cached)
  enrich.rs     museum APIs / Wikidata / Wikipedia
  licensing.rs  the download rules (resources/licensing.json) and the license decision
  sources/      one provider per museum: search, license evidence, image address
  download.rs   the download queue: re-check the license, fetch, verify, save, record
  manifest.rs   appends to manifest.json without disturbing it
  genres.rs     genre list (resources/genres.json) and classification
  tone.rs       brightness measurement and light/dark
  wall.rs       wallpaper, viewer and settings windows
  tray.rs       menu bar item
  platform/     macOS, Windows, Linux desktop-window + wallpaper glue
  cli.rs        command-line control
src/
  shared/       layout math, plate renderer, plate styles, stage (crossfades + pan animation)
  wallpaper/    the desktop window
  viewer/       the full-screen viewer
  settings/     the settings window (React)
```

The slow pan moves the painting one device pixel at a time and sleeps in between (at most 30 updates a second, none while resting at an edge or paused). An infinite CSS animation looks the same but keeps WebKit busy every frame. The pan's phase is tied to when the painting appeared, so restarting the app doesn't send it back to the edge.

## License

MIT for the code. The paintings are yours to choose: Galeria never includes any. Images you download keep the license their museum or author gave them, which Galeria records in your manifest.
