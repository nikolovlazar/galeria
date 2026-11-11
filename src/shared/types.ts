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

export interface Settings {
  libraryPath: string | null;
  wallpaper: WallpaperSettings;
  rotation: { intervalMinutes: number; order: Order; paused: boolean };
  plate: PlateSettings;
  viewer: ViewerSettings;
  general: GeneralSettings;
  excluded: string[];
  favorites: string[];
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
