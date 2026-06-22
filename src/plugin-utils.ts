// SPDX-License-Identifier: AGPL-3.0-or-later
import { invoke } from '@tauri-apps/api/core';
import { notifyError } from './notify';

/**
 * Extract the release channel suffix from a release tag.
 * Returns the lowercased channel name (e.g. "beta", "rc") or null if the
 * tag does not end with a known channel suffix.
 */
export function extractChannel(releaseTag: string): string | null {
  const match = releaseTag.match(/-(test|beta|alpha|rc|dev|preview|canary)$/i);
  return match ? match[1].toLowerCase() : null;
}

/**
 * Export sanitized diagnostics from the backend.
 * Returns the JSON string on success, or undefined on failure (error is
 * reported via system notification).
 */
export async function exportDiagnostics(): Promise<string | undefined> {
  try {
    const data = await invoke<unknown>('export_diagnostics');
    return JSON.stringify(data, null, 2);
  } catch (err) {
    notifyError('导出失败', String(err));
    return undefined;
  }
}
