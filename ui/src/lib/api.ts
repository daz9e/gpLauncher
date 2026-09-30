// Typed calls into the backend commands (src-tauri/src/commands.rs).

import { invoke } from '@tauri-apps/api/core';
import type {
  AddonScope,
  Boot,
  DeviceCode,
  Filters,
  Instance,
  InstanceData,
  InstancePage,
  Item,
  Kind,
  Loader,
  LoaderVersion,
  LogChunk,
  LogFile,
  LogLine,
  Pack,
  PackPage,
  PackVersion,
  Platform,
  Project,
  ProjectPage,
  Settings,
  Snapshot,
  Sort,
  Update,
  Version,
  VersionEntry,
  World,
} from './types';

export const api = {
  boot: () => invoke<Boot>('boot'),
  snapshot: () => invoke<Snapshot>('snapshot'),
  notice: (text: string) => invoke<void>('notice', { text }),
  clearNotice: () => invoke<void>('clear_notice'),

  setSettings: (settings: Settings) => invoke<void>('set_settings', { settings }),
  setCurseforgeKey: (key: string) => invoke<void>('set_curseforge_key', { key }),
  setClientId: (id: string) => invoke<void>('set_client_id', { id }),
  selectAccount: (index: number) => invoke<void>('select_account', { index }),
  removeAccount: (index: number) => invoke<void>('remove_account', { index }),
  addOfflineAccount: (name: string) => invoke<void>('add_offline_account', { name }),
  msRequestCode: () => invoke<DeviceCode>('ms_request_code'),
  msComplete: () => invoke<void>('ms_complete'),
  msCancel: () => invoke<void>('ms_cancel'),

  listVersions: () => invoke<VersionEntry[]>('list_versions'),
  supportedVersions: (loader: Loader) => invoke<string[] | null>('supported_versions', { loader }),
  loaderVersions: (loader: Loader, minecraft: string) =>
    invoke<LoaderVersion[]>('loader_versions', { loader, minecraft }),
  createInstance: (name: string, minecraft: string, loader: Loader, loaderVersion: string) =>
    invoke<Instance>('create_instance', { name, minecraft, loader, loaderVersion }),
  importFiles: (paths: string[]) => invoke<void>('import_files', { paths }),
  installModpack: (pack: Pack, version: PackVersion) => invoke<void>('install_modpack', { pack, version }),
  searchModpacks: (platform: Platform, query: string, filters: Filters, offset: number, limit: number) =>
    invoke<PackPage>('search_modpacks', { platform, query, filters, offset, limit }),
  modpackVersions: (platform: Platform, id: string) => invoke<PackVersion[]>('modpack_versions', { platform, id }),

  launch: (id: string) => invoke<boolean>('launch', { id }),
  kill: (id: string) => invoke<void>('kill', { id }),
  openInstanceWindow: (id: string, page: InstancePage) => invoke<void>('open_instance_window', { id, page }),
  openFolder: (path: string) => invoke<void>('open_folder', { path }),
  openFile: (path: string) => invoke<void>('open_file', { path }),
  exportTarget: (id: string) => invoke<{ dir: string; file_name: string }>('export_target', { id }),
  exportInstance: (id: string, dest: string) => invoke<void>('export_instance', { id, dest }),
  duplicateInstance: (id: string) => invoke<void>('duplicate_instance', { id }),
  deleteInstance: (id: string) => invoke<void>('delete_instance', { id }),
  createShortcut: (id: string) => invoke<string>('create_shortcut', { id }),
  saveInstance: (id: string, data: InstanceData) => invoke<Instance>('save_instance', { id, data }),
  setInstanceIcon: (id: string, path: string) => invoke<void>('set_instance_icon', { id, path }),
  clearInstanceIcon: (id: string) => invoke<void>('clear_instance_icon', { id }),
  javaVersion: (path: string) => invoke<string>('java_version', { path }),
  isFile: (path: string) => invoke<boolean>('is_file', { path }),

  contentList: (id: string, kind: Kind) => invoke<Item[]>('content_list', { id, kind }),
  contentIdentify: (items: Item[]) => invoke<Record<string, Version>>('content_identify', { items }),
  contentSetEnabled: (id: string, kind: Kind, item: Item, enabled: boolean) =>
    invoke<string>('content_set_enabled', { id, kind, item, enabled }),
  contentDelete: (id: string, kind: Kind, item: Item) => invoke<void>('content_delete', { id, kind, item }),
  contentAddFiles: (id: string, kind: Kind, paths: string[]) =>
    invoke<number>('content_add_files', { id, kind, paths }),
  contentCheckUpdates: (id: string, kind: Kind, items: Item[]) =>
    invoke<Update[]>('content_check_updates', { id, kind, items }),
  addonScope: (id: string, kind: Kind) => invoke<AddonScope>('addon_scope', { id, kind }),
  contentInstall: (
    id: string,
    kind: Kind,
    project: Project,
    version: Version | null,
    installed: string[],
    task: string,
  ) => invoke<string>('content_install', { id, kind, project, version, installed, task }),
  contentUpdate: (id: string, kind: Kind, updates: Update[], task: string) =>
    invoke<string>('content_update', { id, kind, updates, task }),
  searchProjects: (
    kind: Kind,
    query: string,
    gameVersion: string | null,
    loaders: string[],
    sort: Sort,
    offset: number,
    limit: number,
  ) => invoke<ProjectPage>('search_projects', { kind, query, gameVersion, loaders, sort, offset, limit }),
  projectVersions: (project: string, loaders: string[], gameVersion: string | null) =>
    invoke<Version[]>('project_versions', { project, loaders, gameVersion }),

  worlds: (id: string) => invoke<World[]>('worlds', { id }),
  dirSize: (path: string) => invoke<number>('dir_size', { path }),
  deleteWorld: (id: string, path: string) => invoke<void>('delete_world', { id, path }),
  screenshots: (id: string) => invoke<string[]>('screenshots', { id }),
  logFiles: (id: string) => invoke<LogFile[]>('log_files', { id }),
  readLog: (id: string, path: string) => invoke<LogLine[]>('read_log', { id, path }),
  latestLog: (id: string) => invoke<LogLine[] | null>('latest_log', { id }),
  getLog: (id: string) => invoke<LogChunk | null>('get_log', { id }),
  uploadLog: (text: string) => invoke<string>('upload_log', { text }),
  quit: () => invoke<void>('quit'),
};

/** Error text of a failed command. */
export function message(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : String(e);
}
