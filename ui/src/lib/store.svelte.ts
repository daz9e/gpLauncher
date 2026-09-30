// The launcher state every window shows: a snapshot from the backend, kept current by `state` events.

import { listen } from '@tauri-apps/api/event';
import { api } from './api';
import type { Boot, Instance, Snapshot } from './types';

class Launcher {
  snap = $state<Snapshot | null>(null);
  boot = $state<Boot | null>(null);
  /** The system prefers dark colors. */
  systemDark = $state(false);
  /** Seconds since the epoch, ticking for play timers and "ago" texts. */
  now = $state(Date.now() / 1000);

  instance(id: string | null | undefined): Instance | undefined {
    return id ? this.snap?.instances.find((i) => i.id === id) : undefined;
  }

  running(id: string): boolean {
    const phase = this.snap?.sessions[id]?.phase;
    return phase === 'preparing' || phase === 'running';
  }

  get dark(): boolean {
    const forced = this.boot?.theme;
    if (forced === 'dark' || forced === 'light') return forced === 'dark';
    const appearance = this.snap?.settings.appearance ?? 'system';
    return appearance === 'system' ? this.systemDark : appearance === 'dark';
  }

  /** Keeps `snap` current with the backend's `state` events. */
  follow() {
    return listen<Snapshot>('state', (e) => (this.snap = e.payload));
  }

  async init(): Promise<void> {
    const media = window.matchMedia?.('(prefers-color-scheme: dark)');
    if (media) {
      this.systemDark = media.matches;
      media.addEventListener?.('change', (e) => (this.systemDark = e.matches));
    }
    await this.follow();
    const [boot, snap] = await Promise.all([api.boot(), api.snapshot()]);
    this.boot = boot;
    this.snap ??= snap;
    setInterval(() => (this.now = Date.now() / 1000), 1000);
  }
}

export const launcher = new Launcher();
