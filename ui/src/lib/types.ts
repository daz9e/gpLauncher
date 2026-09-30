// Shapes of what the backend (src-tauri) sends and takes. Field names follow the Rust structs.

export type Loader = 'vanilla' | 'fabric' | 'quilt' | 'forge' | 'neoforge';
export type Kind = 'mods' | 'resourcepacks' | 'shaderpacks';
export type Platform = 'modrinth' | 'curseforge';
export type Sort = 'relevance' | 'downloads' | 'updated' | 'newest';
export type Appearance = 'system' | 'light' | 'dark';
export type OnLaunch = 'keep_open' | 'minimize';
export type Phase = 'preparing' | 'running' | 'finished';
export type Level = 'launcher' | 'info' | 'warn' | 'error' | 'debug';
export type Progress = [number, number] | null;

export interface Settings {
  data_dir: string;
  game_dir: string;
  memory_mb: number;
  java_path: string;
  jvm_args: string;
  ms_client_id: string;
  curseforge_api_key: string;
  window_width: number | null;
  window_height: number | null;
  fullscreen: boolean;
  appearance: Appearance;
  on_launch: OnLaunch;
}

/** The fields `instance.json` stores; what `save_instance` takes. */
export interface InstanceData {
  name: string;
  minecraft: string;
  loader: Loader;
  loader_version: string;
  memory_mb: number | null;
  jvm_args: string;
  java_path: string;
  window_width: number | null;
  window_height: number | null;
  fullscreen: boolean | null;
  last_played: number;
  play_time: number;
  group: string;
}

export interface Instance extends InstanceData {
  id: string;
  dir: string;
  game_dir: string;
  icon: string | null;
  description: string;
}

export interface Account {
  kind: 'Offline' | 'Microsoft';
  name: string;
  uuid: string;
}

export interface SessionView {
  phase: Phase;
  status: string;
  progress: Progress;
  played: number | null;
  started_at: number;
  key: number;
}

export interface Job {
  active: boolean;
  status: string;
  progress: Progress;
}

export interface Snapshot {
  settings: Settings;
  accounts: Account[];
  selected_account: number;
  instances: Instance[];
  sessions: Record<string, SessionView>;
  job: Job | null;
  manual_downloads: string[];
  running: number;
  busy: boolean;
}

export interface Boot {
  launch: string | null;
  theme: string | null;
  open: string | null;
  env_curseforge_key: boolean;
  os: string;
}

export interface LogLine {
  text: string;
  level: Level;
}

export interface LogChunk {
  id: string;
  key: number;
  start: number;
  lines: LogLine[];
}

export interface VersionEntry {
  id: string;
  kind: string;
  url: string | null;
  sha1: string | null;
  installed: boolean;
}

export interface LoaderVersion {
  version: string;
  stable: boolean;
}

export interface Pack {
  platform: Platform;
  id: string;
  title: string;
  author: string;
  summary: string;
  downloads: number;
  icon_url: string | null;
  website: string | null;
}

export interface PackVersion {
  id: string;
  name: string;
  game_versions: string[];
  loaders: string[];
  url: string | null;
  file_name: string;
  sha1: string | null;
  size: number | null;
}

export interface Filters {
  game_version: string | null;
  loader: Loader | null;
  sort: Sort;
}

export interface PackPage {
  packs: Pack[];
  total: number;
}

export interface Item {
  path: string;
  file_name: string;
  enabled: boolean;
  name: string;
  id: string;
  version: string;
  description: string;
  authors: string[];
  icon: string | null;
  size: number;
  modified: number;
}

export interface Project {
  id: string;
  slug: string;
  title: string;
  author: string;
  summary: string;
  downloads: number;
  icon_url: string | null;
}

export interface Dependency {
  project_id: string | null;
  version_id: string | null;
  kind: string;
}

export interface Version {
  id: string;
  project_id: string;
  name: string;
  number: string;
  game_versions: string[];
  loaders: string[];
  url: string;
  file_name: string;
  sha1: string | null;
  size: number | null;
  dependencies: Dependency[];
}

export interface ProjectPage {
  projects: Project[];
  total: number;
}

export interface Update {
  item: Item;
  current: string | null;
  latest: Version;
}

export interface AddonScope {
  loaders: string[];
  game_version: string | null;
}

export interface World {
  path: string;
  folder: string;
  name: string;
  icon: string | null;
  last_played: number;
  game_mode: string | null;
}

export interface LogFile {
  path: string;
  name: string;
  crash: boolean;
  modified: number;
}

export interface DeviceCode {
  user_code: string;
  verification_uri: string;
}

export interface TaskProgress {
  task: string;
  status: string | null;
  progress: Progress;
}

/** Pages of an instance window. */
export type InstancePage = 'console' | Kind | 'worlds' | 'screenshots' | 'logs' | 'settings';
