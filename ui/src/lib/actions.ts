// Instance actions shared by the grid, the sidebar and the context menu.

import { ask, save } from '@tauri-apps/plugin-dialog';
import { api, message } from './api';
import type { Instance, InstancePage } from './types';

/** Runs a command; a failure shows in the status bar. */
async function attempt(f: () => Promise<unknown>): Promise<void> {
  try {
    await f();
  } catch (e) {
    await api.notice(message(e)).catch(() => {});
  }
}

export const actions = {
  launch: (id: string) => attempt(() => api.launch(id)),
  kill: (id: string) => attempt(() => api.kill(id)),
  open: (id: string, page: InstancePage) => attempt(() => api.openInstanceWindow(id, page)),
  openFolder: (path: string) => attempt(() => api.openFolder(path)),
  duplicate: (id: string) => attempt(() => api.duplicateInstance(id)),
  shortcut: (id: string) => attempt(() => api.createShortcut(id)),

  async export(id: string): Promise<void> {
    await attempt(async () => {
      const { dir, file_name } = await api.exportTarget(id);
      const sep = dir.includes('\\') ? '\\' : '/';
      const dest = await save({
        defaultPath: `${dir}${sep}${file_name}`,
        filters: [{ name: 'Instance archive', extensions: ['zip'] }],
      });
      if (dest) await api.exportInstance(id, dest);
    });
  },

  /** Asks first; returns whether the instance was deleted. */
  async delete(inst: Instance): Promise<boolean> {
    const yes = await ask(
      'The instance folder with its worlds, mods and settings will be deleted permanently.',
      { title: `Delete "${inst.name}"?`, kind: 'warning', okLabel: 'Delete', cancelLabel: 'Cancel' },
    );
    if (!yes) return false;
    try {
      await api.deleteInstance(inst.id);
      return true;
    } catch {
      return false;
    }
  },
};
