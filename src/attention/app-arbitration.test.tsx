// SPDX-License-Identifier: AGPL-3.0-or-later
// §11.2 端到端仲裁测试：更新在设置标签过渡期间到达 → 通知侧播放、固定行不消费；
// 过渡完成后新版本到达 → 固定行播放、通知侧按仲裁不重复提交。
// 使用真实 plugin-updater / local-ai-updater 存储与真实 notify 分发。
import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import App from '../App';
import { checkForPluginUpdates } from '../plugin-updater';
import { checkForLocalAiUpdates } from '../local-ai-updater';
import type { PluginUpdateInfo } from '../types';
import i18n from '../i18n';
import { getAttentionBusState, resetAttentionBusForTests } from './attentionCore';
import { attentionLocalAiUpdateKey, attentionPluginUpdateKey } from './attentionTypes';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

function pluginInfo(pluginId: string, version: string): PluginUpdateInfo {
  return { pluginId, currentVersion: '1.0.0', availableVersion: version, updateAvailable: true };
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

beforeEach(async () => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((command: string) => {
    if (command === 'plugin_updates_check') return Promise.resolve([]);
    if (command === 'local_ai_updates_check') return Promise.resolve([]);
    return Promise.reject(new Error('not mocked'));
  });
  resetAttentionBusForTests();
  await i18n.changeLanguage('zh-CN');
  window.history.replaceState({}, '', '/');
  await checkForPluginUpdates(false);
  await checkForLocalAiUpdates(false);
});

afterEach(() => {
  resetAttentionBusForTests();
});

describe('settings tab transition visibility arbitration (App level)', () => {
  it('plays the plugin update on the notification surface while the plugins tab is still transitioning', async () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    fireEvent.click(screen.getByRole('button', { name: '插件' }));

    invokeMock.mockImplementation((command: string) => {
      if (command === 'plugin_updates_check') return Promise.resolve([pluginInfo('mira-amaster', '2.0.0')]);
      return Promise.reject(new Error('not mocked'));
    });
    await act(async () => { await checkForPluginUpdates(true); });

    const bus = getAttentionBusState();
    expect(bus.active?.scope).toBe('notification:app');
    expect(bus.active?.eventKey).toBe(attentionPluginUpdateKey('mira-amaster', '2.0.0'));
    expect(bus.pending.some((request) => request.scope === 'settings-plugin')).toBe(false);
  });

  it('plays fresh plugin updates on the fixed row after the plugins tab transition completes', async () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    fireEvent.click(screen.getByRole('button', { name: '插件' }));
    await act(async () => { await sleep(170); });

    invokeMock.mockImplementation((command: string) => {
      if (command === 'plugin_updates_check') return Promise.resolve([pluginInfo('mira-amaster', '2.1.0')]);
      return Promise.reject(new Error('not mocked'));
    });
    await act(async () => { await checkForPluginUpdates(true); });

    const bus = getAttentionBusState();
    expect(bus.active?.scope).toBe('settings-plugin');
    expect(bus.active?.eventKey).toBe(attentionPluginUpdateKey('mira-amaster', '2.1.0'));
    expect(bus.pending.some((request) => request.scope === 'notification:app')).toBe(false);
  });

  it('plays the local-ai update on the notification surface while the plugins tab is still transitioning', async () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    fireEvent.click(screen.getByRole('button', { name: '插件' }));

    invokeMock.mockImplementation((command: string) => {
      if (command === 'local_ai_updates_check') {
        return Promise.resolve([
          { component: 'bundle', currentVersion: '1.0.0', availableVersion: '1.2.0', updateAvailable: true },
        ]);
      }
      return Promise.reject(new Error('not mocked'));
    });
    await act(async () => { await checkForLocalAiUpdates(true); });

    const bus = getAttentionBusState();
    expect(bus.active?.scope).toBe('notification:app');
    expect(bus.active?.eventKey).toBe(attentionLocalAiUpdateKey('bundle', '1.2.0'));
    expect(bus.pending.some((request) => request.scope === 'settings-local-ai')).toBe(false);
  });
});