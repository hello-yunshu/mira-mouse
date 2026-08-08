// SPDX-License-Identifier: AGPL-3.0-or-later
// §11.2 / §11.3 组件级测试：设置页标签过渡仲裁与插件 / 本地 AI 安装完成 Flash 门控。
// 使用可注入的伪 plugin-updater / local-ai-updater 存储，断言注意力总线行为。
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { SettingsPage } from '../Settings';
import type { PluginUpdateState } from '../plugin-updater';
import type { LocalAiUpdateState } from '../local-ai-updater';
import type { LocalAiUpdateInfo, PluginUpdateInfo } from '../types';
import i18n from '../i18n';
import { getAttentionBusState, hasAttentionEventPlayedOnce, resetAttentionBusForTests } from './attentionCore';
import {
  ATTENTION_PRIORITY,
  attentionLocalAiInstalledKey,
  attentionLocalAiUpdateKey,
  attentionPluginInstalledKey,
  attentionPluginUpdateKey,
} from './attentionTypes';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const BUNDLED_PLUGIN = {
  pluginId: 'mira-amaster',
  version: '1.0.0',
  asset: 'mira-amaster.mira-plugin',
  sha256: 'test',
  publisherKeyId: 'test',
  releaseTag: 'stable',
  bundleByDefault: true,
  signatureVerified: true,
  evidence: 'test',
  source: 'installed',
};

// ─── 伪存储控制面（vi.mock 工厂与测试共享）─────────────────────────────────
const plugin = vi.hoisted(() => {
  const listeners = new Set<(next: unknown) => void>();
  let state: unknown = { phase: 'idle', updates: [], downloadedBytes: 0 };
  return {
    setState(next: unknown): void {
      state = next;
      for (const listener of [...listeners]) listener(state);
    },
    current(): unknown {
      return state;
    },
    reset(): void {
      state = { phase: 'idle', updates: [], downloadedBytes: 0 };
      listeners.clear();
    },
    subscribe(listener: (next: unknown) => void): () => void {
      listeners.add(listener);
      listener(state);
      return () => listeners.delete(listener);
    },
  };
});

const localAi = vi.hoisted(() => {
  const listeners = new Set<(next: unknown) => void>();
  let state: unknown = { phase: 'idle', updates: [], downloadedBytes: 0 };
  return {
    setState(next: unknown): void {
      state = next;
      for (const listener of [...listeners]) listener(state);
    },
    current(): unknown {
      return state;
    },
    subscribe(listener: (next: unknown) => void): () => void {
      listeners.add(listener);
      listener(state);
      return () => listeners.delete(listener);
    },
    reset(): void {
      state = { phase: 'idle', updates: [], downloadedBytes: 0 };
      listeners.clear();
    },
  };
});

vi.mock('../plugin-updater', () => ({
  pluginUpdateState: () => plugin.current(),
  onPluginUpdateState: (listener: unknown) => plugin.subscribe(listener as (next: unknown) => void),
  checkForPluginUpdates: vi.fn(),
  installPluginUpdate: vi.fn(),
  startAutomaticPluginUpdateCheck: vi.fn(),
}));
vi.mock('../local-ai-updater', () => ({
  localAiUpdateState: () => localAi.current(),
  onLocalAiUpdateState: (listener: unknown) => localAi.subscribe(listener as (next: unknown) => void),
  checkForLocalAiUpdates: vi.fn(),
  installLocalAiUpdate: vi.fn(),
  rollbackLocalAiUpdate: vi.fn(),
  localAiComponentLabel: (component: string) => component,
  startAutomaticLocalAiUpdateCheck: vi.fn(),
  stopAutomaticLocalAiUpdateCheck: vi.fn(),
}));

function emitPlugin(next: PluginUpdateState): void {
  act(() => plugin.setState(next));
}

function emitLocalAi(next: Partial<LocalAiUpdateState> & { updates: LocalAiUpdateInfo[] }): void {
  act(() => localAi.setState(next));
}

function pluginInfo(pluginId: string, version: string): PluginUpdateInfo {
  return { pluginId, currentVersion: '1.0.0', availableVersion: version, updateAvailable: true };
}

function localAiBundle(version: string): LocalAiUpdateInfo {
  return { component: 'runtime', currentVersion: '1.0.0', availableVersion: version, updateAvailable: true };
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

beforeEach(async () => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((command: string) => {
    if (command === 'about_info') return Promise.resolve({ bundledPlugins: [BUNDLED_PLUGIN] });
    return Promise.reject(new Error(`not mocked: ${command}`));
  });
  plugin.reset();
  localAi.reset();
  resetAttentionBusForTests();
  await i18n.changeLanguage('zh-CN');
});

afterEach(() => {
  resetAttentionBusForTests();
});

function renderSettings(initialTab: 'general' | 'plugins', onTabChange: (tab: string) => void, previewMode = true) {
  render(
    <SettingsPage
      previewMode={previewMode}
      initialTab={initialTab}
      onTabChange={onTabChange}
      onNavigateAbout={vi.fn()}
      onThemeChange={vi.fn()}
    />,
  );
}

describe('settings tab transition and update arbitration', () => {
  it('keeps the parent visible tab on general while the transition runs and does not consume incoming updates', async () => {
    const onTabChange = vi.fn();
    renderSettings('general', onTabChange);
    await waitFor(() => expect(onTabChange).toHaveBeenLastCalledWith('general'));

    fireEvent.click(screen.getByRole('button', { name: '插件' }));

    emitPlugin({ phase: 'available', updates: [pluginInfo('mira-amaster', '2.0.0')], downloadedBytes: 0 });

    expect(onTabChange).toHaveBeenLastCalledWith('general');
    expect(getAttentionBusState().active).toBeNull();
    expect(hasAttentionEventPlayedOnce(attentionPluginUpdateKey('mira-amaster', '2.0.0'))).toBe(false);

    await act(async () => { await sleep(210); });
    expect(onTabChange).toHaveBeenLastCalledWith('plugins');
    // 同一版本在过渡后才可见也不会“补强调”。
    expect(getAttentionBusState().active).toBeNull();
  });

  it('plays a fresh plugin update on the fixed row once the plugins tab is visible', async () => {
    const onTabChange = vi.fn();
    renderSettings('general', onTabChange, false);
    await waitFor(() => expect(onTabChange).toHaveBeenLastCalledWith('general'));

    fireEvent.click(screen.getByRole('button', { name: '插件' }));
    await act(async () => { await sleep(210); });
    await screen.findByText('mira-amaster');

    emitPlugin({ phase: 'available', updates: [pluginInfo('mira-amaster', '2.1.0')], downloadedBytes: 0 });

    const bus = getAttentionBusState();
    expect(bus.active?.scope).toBe('settings-plugin');
    expect(bus.active?.eventKey).toBe(attentionPluginUpdateKey('mira-amaster', '2.1.0'));
    expect(bus.active?.priority).toBe(ATTENTION_PRIORITY['update-available']);
    expect(bus.pending.length).toBe(0);
    const beam = document.querySelector(`[data-event-key="${attentionPluginUpdateKey('mira-amaster', '2.1.0')}"]`);
    expect(beam).toHaveClass('attention-beam--line');
    expect(beam?.parentElement).toHaveClass('plugin-update-row');
  });

  it('plays a fresh local-ai update on its fixed update row', async () => {
    const onTabChange = vi.fn();
    renderSettings('plugins', onTabChange);

    emitLocalAi({ phase: 'available', updates: [localAiBundle('1.2.0')], downloadedBytes: 0 });

    const eventKey = attentionLocalAiUpdateKey('runtime', '1.2.0');
    expect(getAttentionBusState().active?.eventKey).toBe(eventKey);
    const beam = document.querySelector(`[data-event-key="${eventKey}"]`);
    expect(beam).toHaveClass('attention-beam--line');
    expect(beam?.parentElement).toHaveClass('plugin-update-row');
  });

  it('ignores a fresh local-ai version while the plugins tab is not visible', async () => {
    const onTabChange = vi.fn();
    renderSettings('general', onTabChange);
    await waitFor(() => expect(onTabChange).toHaveBeenLastCalledWith('general'));

    fireEvent.click(screen.getByRole('button', { name: '插件' }));

    emitLocalAi({ phase: 'available', updates: [localAiBundle('1.1.0')], downloadedBytes: 0 });
    emitLocalAi({ phase: 'available', updates: [localAiBundle('1.2.0')], downloadedBytes: 0 });

    expect(onTabChange).toHaveBeenLastCalledWith('general');
    expect(getAttentionBusState().active).toBeNull();
    expect(hasAttentionEventPlayedOnce(attentionLocalAiUpdateKey('runtime', '1.2.0'))).toBe(false);

    await act(async () => { await sleep(210); });
    expect(onTabChange).toHaveBeenLastCalledWith('plugins');
  });

  it('plays a plugin flash as installed only when the plugins tab is visible', async () => {
    const onTabChange = vi.fn();
    renderSettings('plugins', onTabChange, false);
    await screen.findByText('mira-amaster');

    emitPlugin({
      phase: 'installed',
      updates: [],
      downloadedBytes: 0,
      lastInstalledPluginId: 'mira-amaster',
      lastInstalledVersion: '2.0.0',
    });

    const bus = getAttentionBusState();
    expect(bus.active?.scope).toBe('settings-plugin');
    expect(bus.active?.variant).toBe('flash');
    expect(bus.active?.eventKey).toBe(attentionPluginInstalledKey('mira-amaster', '2.0.0'));
    expect(bus.active?.priority).toBe(ATTENTION_PRIORITY['update-installed']);
    expect(bus.active?.durationMs).toBe(950);
    expect(bus.pending.length).toBe(0);
    const beam = document.querySelector(`[data-event-key="${attentionPluginInstalledKey('mira-amaster', '2.0.0')}"]`);
    expect(beam).toHaveClass('attention-beam--flash');
    expect(beam?.parentElement).toHaveClass('plugin-item');
    expect(beam?.closest('.plugin-update-row')).toBeNull();
  });

  it('does not submit or consume the flash when installations finish while general is visible, and switching later never replays', async () => {
    const onTabChange = vi.fn();
    renderSettings('general', onTabChange);
    await waitFor(() => expect(onTabChange).toHaveBeenLastCalledWith('general'));

    emitPlugin({
      phase: 'installed',
      updates: [],
      downloadedBytes: 0,
      lastInstalledPluginId: 'mira-amaster',
      lastInstalledVersion: '2.0.0',
    });

    expect(getAttentionBusState().active).toBeNull();
    expect(hasAttentionEventPlayedOnce(attentionPluginInstalledKey('mira-amaster', '2.0.0'))).toBe(false);

    fireEvent.click(screen.getByRole('button', { name: '插件' }));
    await act(async () => { await sleep(210); });

    // phase 已处于 installed，其他依赖变化不触发重播。
    emitPlugin({ phase: 'available', updates: [], downloadedBytes: 0 });
    expect(getAttentionBusState().active).toBeNull();
    expect(hasAttentionEventPlayedOnce(attentionPluginInstalledKey('mira-amaster', '2.0.0'))).toBe(false);
  });

  it('plays a local-ai flash as installed only when the plugins tab is visible', async () => {
    const onTabChange = vi.fn();
    // 版本在进入设置前已可用（固定行挂载时仅初始化、不强调），安装完成时才触发 flash。
    emitLocalAi({ phase: 'available', updates: [localAiBundle('1.1.0')], downloadedBytes: 0 });
    renderSettings('plugins', onTabChange);

    expect(getAttentionBusState().active).toBeNull();

    emitLocalAi({
      phase: 'installed',
      updates: [{ ...localAiBundle('1.1.0'), updateAvailable: false, currentVersion: '1.1.0' }],
      downloadedBytes: 0,
      updatedComponents: ['runtime'],
    });

    const bus = getAttentionBusState();
    expect(bus.active?.scope).toBe('settings-local-ai');
    expect(bus.active?.variant).toBe('flash');
    expect(bus.active?.eventKey).toBe(attentionLocalAiInstalledKey('runtime', '1.1.0'));
    expect(bus.active?.priority).toBe(ATTENTION_PRIORITY['update-installed']);
    expect(bus.pending.length).toBe(0);
    const beam = document.querySelector(`[data-event-key="${attentionLocalAiInstalledKey('runtime', '1.1.0')}"]`);
    expect(beam).toHaveClass('attention-beam--flash');
    expect(beam?.parentElement).toHaveClass('plugin-item');
    expect(beam?.closest('.plugin-update-row')).toBeNull();
  });

  it('does not submit a local-ai flash while general is visible', async () => {
    const onTabChange = vi.fn();
    renderSettings('general', onTabChange);
    await waitFor(() => expect(onTabChange).toHaveBeenLastCalledWith('general'));

    emitLocalAi({ phase: 'installed', updates: [localAiBundle('1.1.0')], downloadedBytes: 0, updatedComponents: ['runtime'] });

    expect(getAttentionBusState().active).toBeNull();
    expect(hasAttentionEventPlayedOnce(attentionLocalAiInstalledKey('runtime', '1.1.0'))).toBe(false);
  });
});
