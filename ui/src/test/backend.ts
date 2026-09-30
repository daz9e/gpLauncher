// A fake backend for component tests: answers commands like src-tauri does and records calls.

import { clearMocks, mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import { cleanup } from '@testing-library/svelte';
import { launcher } from '../lib/store.svelte';
import type { Boot, Instance, Snapshot } from '../lib/types';

export type Handler = (args: Record<string, unknown>) => unknown;

export function instance(over: Partial<Instance> = {}): Instance {
  const id = over.id ?? over.name ?? 'inst';
  return {
    id,
    dir: `/data/instances/${id}`,
    game_dir: `/data/instances/${id}/minecraft`,
    icon: null,
    description: '1.21.1',
    name: id,
    minecraft: '1.21.1',
    loader: 'vanilla',
    loader_version: '',
    memory_mb: null,
    jvm_args: '',
    java_path: '',
    window_width: null,
    window_height: null,
    fullscreen: null,
    last_played: 0,
    play_time: 0,
    group: '',
    ...over,
  };
}

export function snapshot(over: Partial<Snapshot> = {}): Snapshot {
  return {
    settings: {
      data_dir: '/data',
      game_dir: '',
      memory_mb: 4096,
      java_path: '',
      jvm_args: '',
      ms_client_id: '',
      curseforge_api_key: '',
      window_width: null,
      window_height: null,
      fullscreen: false,
      appearance: 'system',
      on_launch: 'keep_open',
    },
    accounts: [],
    selected_account: 0,
    instances: [],
    sessions: {},
    job: null,
    manual_downloads: [],
    running: 0,
    busy: false,
    ...over,
  };
}

export const BOOT: Boot = { launch: null, theme: null, open: null, env_curseforge_key: false, os: 'macos' };

export class Backend {
  calls: { cmd: string; args: Record<string, unknown> }[] = [];
  handlers: Record<string, Handler> = {};
  snap: Snapshot;

  constructor(snap: Snapshot, handlers: Record<string, Handler> = {}, window = 'main') {
    this.snap = snap;
    this.handlers = {
      boot: () => BOOT,
      snapshot: () => this.snap,
      clear_notice: () => null,
      notice: () => null,
      ...handlers,
    };
    mockWindows(window);
    mockIPC(
      (cmd, payload) => {
        const args = (payload ?? {}) as Record<string, unknown>;
        this.calls.push({ cmd, args });
        const handler = this.handlers[cmd];
        if (handler) return handler(args);
        // Plugin commands the tests do not care about.
        if (cmd.startsWith('plugin:')) return null;
        throw new Error(`unexpected command ${cmd}`);
      },
      { shouldMockEvents: true },
    );
    launcher.snap = snap;
    launcher.boot = BOOT;
    launcher.follow();
  }

  /** Answers every question dialog with the button labelled `label`. */
  answer(label: string) {
    this.handlers['plugin:dialog|message'] = () => label;
  }

  called(cmd: string) {
    return this.calls.filter((c) => c.cmd === cmd).map((c) => c.args);
  }

  /** Sends a new snapshot as the backend's emitter would. */
  async push(snap: Snapshot) {
    this.snap = snap;
    await emit('state', snap);
  }

  /** Unmounts first: components unlisten from the mocked events on the way out. */
  static async reset() {
    cleanup();
    await settle();
    clearMocks();
    launcher.snap = null;
    launcher.boot = null;
  }
}

/** Lets pending promises and effects run. */
export async function settle(times = 5) {
  for (let i = 0; i < times; i++) await new Promise((r) => setTimeout(r, 0));
}
