// SPDX-License-Identifier: AGPL-3.0-or-later
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AboutInfo } from './types';
import {
  loadAboutInfo,
  peekAboutInfo,
  resetRuntimeDataCacheForTests,
} from './runtime-data-cache';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

function about(version: string): AboutInfo {
  return {
    name: 'Mira Mouse',
    version,
    identifier: 'run.hey.mira',
    platform: 'windows',
    architecture: 'x86_64',
    rustVersion: 'test',
    buildDate: 'test',
    gitCommit: 'test',
    bundledPlugins: [],
    contact: {},
    updaterActive: true,
  };
}

describe('runtime data cache', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    resetRuntimeDataCacheForTests();
  });

  it('deduplicates concurrent native reads and reuses the successful snapshot', async () => {
    let resolveRequest!: (value: AboutInfo) => void;
    invokeMock.mockReturnValue(new Promise<AboutInfo>((resolve) => { resolveRequest = resolve; }));

    const first = loadAboutInfo();
    const second = loadAboutInfo();
    expect(invokeMock).toHaveBeenCalledTimes(1);

    resolveRequest(about('1.1.8'));
    await expect(Promise.all([first, second])).resolves.toEqual([about('1.1.8'), about('1.1.8')]);
    expect(peekAboutInfo()?.version).toBe('1.1.8');

    await expect(loadAboutInfo()).resolves.toEqual(about('1.1.8'));
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('keeps the last good snapshot when an explicit refresh fails', async () => {
    invokeMock.mockResolvedValueOnce(about('1.1.8'));
    await loadAboutInfo();
    invokeMock.mockRejectedValueOnce(new Error('temporarily busy'));

    await expect(loadAboutInfo({ refresh: true })).rejects.toThrow('temporarily busy');
    expect(peekAboutInfo()?.version).toBe('1.1.8');
  });
});
