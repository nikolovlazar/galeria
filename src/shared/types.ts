// Mirrors the serde (camelCase) structures in src-tauri/src.

export type FitMode = "auto" | "fit" | "fill" | "pan";
export type Matte = "blur" | "color" | "average";
export type FrameStyle = "none" | "black" | "white" | "gilded" | "walnut";
export type Order = "shuffle" | "sequential";

export interface PlatePosition {
  x: number;
  y: number;
}

export interface WallpaperSettings {
  live: boolean;
  syncSystem: boolean;
  fit: FitMode;
  cropTolerance: number;
  pan: boolean;
  panSpeed: number;
  panPause: number;
  panPaused: boolean;
  matte: Matte;
  matteColor: string;
  frame: FrameStyle;
  fitMargin: number;
  shadow: boolean;
  spotlight: boolean;
  sameOnAllDisplays: boolean;
  matchOrientation: boolean;
  transitionSeconds: number;
}

export interface PlateFields {
  artist: boolean;
  artistBio: boolean;
  date: boolean;
  medium: boolean;
  dimensions: boolean;
  description: boolean;
  museum: boolean;
  credit: boolean;
  license: boolean;
  accession: boolean;
  qr: boolean;
}

export interface PlateSettings {
  visible: boolean;
  style: string;
  scale: number;
  width: number;
  fields: PlateFields;
  descriptionLines: number;
  defaultPosition: PlatePosition;
  positions: Record<string, PlatePosition>;
  autoHideSeconds: number;
  foreignDescriptions: boolean;
  margin: number;
}

export interface ViewerSettings {
  showPlate: boolean;
  platePosition: PlatePosition;
  hideCursor: boolean;
  smoothing: number;
  minimap: boolean;
  edgeDeadZone: number;
}

export interface GeneralSettings {
  showInDock: boolean;
  launchAtLogin: boolean;
  enrichOnline: boolean;
  restoreWallpaperOnQuit: boolean;
}

export type ScopeKind = "all" | "favorites" | "playlist" | "filter" | "selection";

/** Which paintings take part in the rotation. */
export interface Scope {
  kind: ScopeKind;
  playlist: string | null;
  /** Empty = any museum. */
  museums: string[];
  /** Genre ids; empty = any genre. */
  genres: string[];
  selection: string[];
}

export interface Playlist {
  id: string;
  name: string;
  items: string[];
}

export type Tone = "light" | "dark";
export type ToneOverride = "light" | "dark" | "both";

export interface AppearanceSettings {
  matchSystem: boolean;
  /** Brightness (0–255) from which a painting counts as light; null = the library's median. */
  threshold: number | null;
  overrides: Record<string, ToneOverride>;
  switchImmediately: boolean;
}

export interface DownloadSettings {
  maxLongEdge: number;
  landscapeOnly: boolean;
  minLongEdge: number;
}

export interface Settings {
  libraryPath: string | null;
  wallpaper: WallpaperSettings;
  rotation: { intervalMinutes: number; order: Order; paused: boolean; scope: Scope };
  plate: PlateSettings;
  viewer: ViewerSettings;
  general: GeneralSettings;
  excluded: string[];
  favorites: string[];
  sources: { disabledMuseums: string[] };
  playlists: Playlist[];
  appearance: AppearanceSettings;
  download: DownloadSettings;
}

export interface Painting {
  id: string;
  file: string;
  title: string;
  originalTitle: string | null;
  artist: string | null;
  artistBio: string | null;
  date: string | null;
  medium: string | null;
  dimensions: string | null;
  description: string | null;
  descriptionSource: string | null;
  descriptionLicense: string | null;
  descriptionLang: string | null;
  didYouKnow: string | null;
  culture: string | null;
  museum: string | null;
  credit: string | null;
  creditLine: string | null;
  accession: string | null;
  license: string | null;
  /** The image's license (CC BY, CC BY-SA) requires naming its author. */
  attributionRequired: boolean;
  source: string | null;
  sourcePage: string | null;
  wikipediaUrl: string | null;
  tags: string[];
  kind: string | null;
  width: number;
  height: number;
  aspect: number;
  orientation: "landscape" | "portrait" | "square";
  bytes: number;
  excluded: boolean;
  favorite: boolean;
  enriched: boolean;
  /** Genre ids. */
  genres: string[];
  /** Mean luma 0–255. */
  brightness: number | null;
  tone: Tone | null;
  toneOverride: ToneOverride | null;
  museumEnabled: boolean;
  inRotation: boolean;
  extra: Record<string, unknown>;
}

export interface Insets {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

export interface Display {
  key: string;
  name: string;
  label: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
  primary: boolean;
  insets: Insets;
  painting: Painting | null;
  shownAt: number;
  nextChangeAt: number | null;
  historyLen: number;
}

export interface LibrarySummary {
  root: string | null;
  error: string | null;
  manifestFound: boolean;
  entries: number;
  valid: number;
  eligible: number;
  missing: number;
  invalid: number;
  duplicates: number;
  unindexed: number;
  excluded: number;
  generated: string | null;
  loadedAt: number;
  enriched: number;
  enrichBusy: boolean;
}

export interface CustomStyle {
  id: string;
  name: string;
  css: string;
  path: string;
}

export interface Snapshot {
  settings: Settings;
  displays: Display[];
  arrange: boolean;
  library: LibrarySummary;
  builtinStyles: { id: string; name: string }[];
  customStyles: CustomStyle[];
  stylesDir: string;
  platform: "macos" | "windows" | "linux";
  liveSupported: boolean;
  version: string;
  originalWallpapers: number;
  theme: Tone | null;
  tone: { threshold: number; autoThreshold: number; light: number; dark: number; unknown: number };
  rotation: { count: number; matching: number; empty: boolean };
  museums: MuseumSummary[];
  genres: { id: string; name: string; count: number }[];
  downloads: { active: number; done: number; failed: number };
}

/** A museum named in the manifest, with how Galeria treats it. */
export interface MuseumSummary {
  name: string;
  count: number;
  enabled: boolean;
  /** Provider id when Galeria can download from this museum directly. */
  source: string | null;
  direct: boolean;
  program: string | null;
  license: string | null;
  summary: string;
  policyUrl: string | null;
}

// --- Licensing rules (src-tauri/resources/licensing.json, as compiled into the app) ---

export interface LicenseRule {
  id: string;
  label: string;
  allowed: boolean;
  attribution: boolean;
  share_alike: boolean;
  url: string | null;
  reason: string | null;
  obligations: string[];
}

export interface SourceRule {
  id: string;
  name: string;
  museum: string | null;
  program: string;
  grants: string | null;
  policy_url: string;
  data_url: string;
  quote: string;
  summary: string;
  evidence: string;
  attribution: { required: boolean | null; requested: string | null; credit: string | null; note: string | null };
  images: string;
  etiquette: { search_concurrency: number; download_interval_ms: number; note: string | null };
}

export interface Rules {
  version: number;
  reviewed: string;
  policy: string;
  disclaimer: string;
  licenses: LicenseRule[];
  sources: SourceRule[];
  museums: { name: string; match: string[]; summary: string; policy_url: string | null; license: string | null }[];
  fallback: { direct: boolean; via: string; summary: string };
}

export interface Genre {
  id: string;
  name: string;
  tags: string[];
  /** What to type into a museum's search box to browse this genre. */
  search: string;
}

export interface CatalogStatus {
  source: string;
  ready: boolean;
  builtAt: number | null;
  count: number;
  stale: boolean;
  sizeHint: string;
}

export interface Catalogue {
  rules: Rules;
  genres: Genre[];
  catalogs: CatalogStatus[];
}

// --- Discover & downloads ---

/** The verdict on one record, made from the museum's own data. */
export interface Decision {
  allowed: boolean;
  licenseId: string | null;
  license: string;
  licenseUrl: string | null;
  attributionRequired: boolean;
  shareAlike: boolean;
  obligations: string[];
  reason: string;
  evidence: string;
  program: string;
  policyUrl: string;
}

export interface Candidate {
  id: string;
  source: string;
  sourceId: string;
  locator: string;
  title: string;
  titleOriginal: string | null;
  artist: string | null;
  artistBio: string | null;
  date: string | null;
  medium: string | null;
  dimensions: string | null;
  description: string | null;
  museum: string;
  creditLine: string | null;
  accession: string | null;
  department: string | null;
  pageUrl: string | null;
  thumbUrl: string | null;
  previewUrl: string | null;
  width: number | null;
  height: number | null;
  aspect: number | null;
  /** Size of the download, when the museum states it. */
  downloadBytes: number | null;
  genres: string[];
  decision: Decision | null;
  inLibrary: boolean;
  notes: string[];
}

export interface DiscoverQuery {
  source: string;
  text: string;
  cursor?: string | null;
  museum?: string | null;
  landscapeOnly: boolean;
  minLongEdge: number;
}

export interface DiscoverPage {
  items: Candidate[];
  total: number | null;
  next: string | null;
  notice: string | null;
  needsCatalog: boolean;
  hidden: number;
}

export type JobState =
  | { state: "queued" }
  | { state: "checking" }
  | { state: "downloading"; received: number; total: number | null }
  | { state: "processing" }
  | { state: "done"; file: string; width: number; height: number; bytes: number; license: string }
  | { state: "failed"; error: string }
  | { state: "skipped"; reason: string };

export type Job = {
  id: string;
  source: string;
  locator: string;
  title: string;
  artist: string | null;
  thumbUrl: string | null;
} & JobState;

export interface CatalogProgress {
  source: string;
  stage: string;
  received: number;
  total: number;
}

export interface Issue {
  file: string;
  problem: string;
}

export interface LibraryView {
  paintings: Painting[];
  missing: Issue[];
  invalid: Issue[];
  duplicates: Issue[];
  unindexed: string[];
}

export interface Colors {
  average: string;
  wall: string;
}

/** Recursive partial, for settings patches. */
export type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? (T[K] extends unknown[] ? T[K] : DeepPartial<T[K]>) : T[K] };
