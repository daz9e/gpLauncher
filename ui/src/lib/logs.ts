// A console's lines: the live log of a game, merged from `log` event chunks, or fixed lines.

import type { Level, LogChunk, LogLine } from './types';

export const MAX_LINES = 100_000;

export type Severity = 'all' | 'warnings' | 'errors';

export class LogBuffer {
  lines: LogLine[] = [];
  /** Absolute index of `lines[0]`. */
  start = 0;
  /** The launch the lines belong to; a new one starts over. */
  key: number | null = null;

  /** Replaces everything with a full chunk (from `get_log`). */
  reset(chunk: LogChunk | null): void {
    this.key = chunk?.key ?? null;
    this.start = chunk?.start ?? 0;
    this.lines = chunk ? chunk.lines.slice() : [];
    this.trim();
  }

  setLines(lines: LogLine[]): void {
    this.key = null;
    this.start = 0;
    this.lines = lines;
  }

  get end(): number {
    return this.start + this.lines.length;
  }

  /**
   * Appends an event chunk. Returns false when it does not connect to what is here (another
   * launch, or lines were missed); the caller then fetches the whole log again.
   */
  append(chunk: LogChunk): boolean {
    if (chunk.key !== this.key) return false;
    const chunkEnd = chunk.start + chunk.lines.length;
    if (chunk.start > this.end) return false;
    if (chunkEnd <= this.end) return true;
    this.lines.push(...chunk.lines.slice(this.end - chunk.start));
    this.trim();
    return true;
  }

  private trim(): void {
    const extra = this.lines.length - MAX_LINES;
    if (extra > 0) {
      this.lines.splice(0, extra);
      this.start += extra;
    }
  }
}

export function passes(level: Level, severity: Severity): boolean {
  if (severity === 'all') return true;
  if (severity === 'warnings') return level === 'warn' || level === 'error';
  return level === 'error';
}

/** Indices of lines passing the severity and text filter; `null` = no filter, all lines. */
export function filterLines(lines: LogLine[], severity: Severity, query: string): number[] | null {
  const q = query.trim().toLowerCase();
  if (severity === 'all' && !q) return null;
  const out: number[] = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (passes(line.level, severity) && (!q || line.text.toLowerCase().includes(q))) out.push(i);
  }
  return out;
}
