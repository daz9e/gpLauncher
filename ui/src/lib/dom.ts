// Helpers for images and files shown from the launcher folder.

import { convertFileSrc } from '@tauri-apps/api/core';

/** URL of a local file for `<img src>`, through Tauri's asset protocol. */
export function fileUrl(path: string): string {
  try {
    return convertFileSrc(path);
  } catch {
    return path;
  }
}
