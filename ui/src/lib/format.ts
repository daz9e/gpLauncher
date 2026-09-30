// Text helpers shared by the windows.

import type { Instance, Kind, Loader } from './types';

/** "3 minutes ago", "Yesterday", "12 days ago"; `unix` in seconds, 0 = never. */
export function ago(unix: number, now: number = Date.now() / 1000): string {
  if (!unix) return 'Never';
  const secs = Math.max(0, Math.floor(now - unix));
  const plural = (n: number, unit: string) => (n === 1 ? `1 ${unit} ago` : `${n} ${unit}s ago`);
  if (secs < 60) return 'Just now';
  if (secs < 3600) return plural(Math.floor(secs / 60), 'minute');
  if (secs < 86400) return plural(Math.floor(secs / 3600), 'hour');
  if (secs < 172800) return 'Yesterday';
  if (secs < 86400 * 60) return plural(Math.floor(secs / 86400), 'day');
  if (secs < 86400 * 730) return plural(Math.floor(secs / (86400 * 30)), 'month');
  return plural(Math.floor(secs / (86400 * 365)), 'year');
}

/** `1h 05m`, `12m`, `40s`. */
export function duration(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  return `${Math.floor(s / 3600)}h ${String(Math.floor(s / 60) % 60).padStart(2, '0')}m`;
}

/** `17.7M`, `950K`, `42`. */
export function shortNumber(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${Math.round(n / 1e3)}K`;
  return `${(n / 1e6).toFixed(1)}M`;
}

/** Same as the core's `human_size`. */
export function humanSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1_048_576) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1_073_741_824) return `${(bytes / 1_048_576).toFixed(1)} MB`;
  return `${(bytes / 1_073_741_824).toFixed(2)} GB`;
}

/** Summaries often span several lines; lists show them as one. */
export function oneLine(text: string): string {
  return text.split(/\s+/).filter(Boolean).join(' ');
}

export const LOADER_LABELS: Record<Loader, string> = {
  vanilla: 'Vanilla',
  fabric: 'Fabric',
  quilt: 'Quilt',
  forge: 'Forge',
  neoforge: 'NeoForge',
};

export function capitalize(s: string): string {
  return s === 'neoforge' ? 'NeoForge' : s.charAt(0).toUpperCase() + s.slice(1);
}

/** `1.21.1 · Fabric`, without the loader version. */
export function shortDescription(inst: Instance): string {
  return inst.loader === 'vanilla' ? inst.minecraft : `${inst.minecraft} · ${LOADER_LABELS[inst.loader]}`;
}

export function loaderText(inst: Instance): string {
  if (inst.loader === 'vanilla') return 'None';
  const label = LOADER_LABELS[inst.loader];
  return inst.loader_version ? `${label} ${inst.loader_version}` : `${label} (latest)`;
}

export const KIND_LABELS: Record<Kind, string> = {
  mods: 'Mods',
  resourcepacks: 'Resource packs',
  shaderpacks: 'Shader packs',
};

export const KIND_NOUNS: Record<Kind, string> = {
  mods: 'mod',
  resourcepacks: 'resource pack',
  shaderpacks: 'shader pack',
};

export const KIND_ICONS: Record<Kind, string> = {
  mods: 'puzzle',
  resourcepacks: 'image',
  shaderpacks: 'sparkles',
};

export const MODRINTH_TYPES: Record<Kind, string> = {
  mods: 'mod',
  resourcepacks: 'resourcepack',
  shaderpacks: 'shader',
};

/** "1 mod", "3 resource packs". */
export function count(n: number, kind: Kind): string {
  return `${n} ${KIND_NOUNS[kind]}${n === 1 ? '' : 's'}`;
}

export function versionKindLabel(kind: string): string {
  return { release: 'Release', snapshot: 'Snapshot', old_beta: 'Beta', old_alpha: 'Alpha' }[kind] ?? '';
}

/** Memory preset label: `4 GB`, `1536 MB`. */
export function memoryLabel(mb: number): string {
  return mb % 1024 === 0 ? `${mb / 1024} GB` : `${mb} MB`;
}

export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export function fileStem(path: string): string {
  const name = fileName(path);
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(0, dot) : name;
}

/** Whether a status line reports a failure. */
export function isError(status: string): boolean {
  return status.startsWith('Error') || status.startsWith('Game crashed');
}

export const MIN_MEMORY_MB = 512;
export const MEMORY_PRESETS = [2048, 4096, 6144, 8192];

/** Parses an optional positive size; `undefined` = invalid. */
function size(text: string): number | null | undefined {
  const t = text.trim();
  if (!t) return null;
  if (!/^\d+$/.test(t)) return undefined;
  const v = Number(t);
  return v > 0 ? v : undefined;
}

/** Window width and height fields: both or neither. */
export function parseResolution(
  width: string,
  height: string,
): { ok: true; width: number | null; height: number | null } | { ok: false; error: string } {
  const w = size(width);
  const h = size(height);
  if (w === undefined || h === undefined) return { ok: false, error: 'Width and height are numbers of pixels' };
  if ((w === null) !== (h === null)) return { ok: false, error: 'Set both width and height, or neither' };
  return { ok: true, width: w, height: h };
}

/** Memory field: at least [`MIN_MEMORY_MB`]; empty = `null`. */
export function parseMemory(text: string): { ok: true; mb: number | null } | { ok: false; error: string } {
  const t = text.trim();
  if (!t) return { ok: true, mb: null };
  const mb = /^\d+$/.test(t) ? Number(t) : NaN;
  if (!(mb >= MIN_MEMORY_MB)) return { ok: false, error: `At least ${MIN_MEMORY_MB} MB` };
  return { ok: true, mb };
}

export function validOfflineName(name: string): boolean {
  return /^[A-Za-z0-9_]{3,16}$/.test(name);
}
