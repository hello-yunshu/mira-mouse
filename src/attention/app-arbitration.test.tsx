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
import { notifySuccess } from '../notify';
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
    await act(async () => { await sleep(210); });

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

describe('notification beam binding (P1-3)', () => {
  function mockPluginUpdate(version: string): void {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'plugin_updates_check') return Promise.resolve([pluginInfo('mira-amaster', version)]);
      return Promise.reject(new Error('not mocked'));
    });
  }

  it('更新通知 A 的 Beam 不会“套到”普通成功通知 B 上，A 仍可自然到期', async () => {
    render(<App />);
    mockPluginUpdate('2.0.0');
    await act(async () => { await checkForPluginUpdates(true); });

    const beamA = attentionPluginUpdateKey('mira-amaster', '2.0.0');
    expect(getAttentionBusState().active?.eventKey).toBe(beamA);
    let card = document.querySelector('.app-notification');
    expect(card?.querySelector(`[data-event-key="${beamA}"]`)).not.toBeNull();

    await act(async () => { notifySuccess('搞定啦'); });
    card = document.querySelector('.app-notification');
    expect(card).not.toBeNull();
    expect(card?.textContent).toContain('搞定啦');
    // 普通通知 B 不渲染 A 的 Beam，也不渲染任何 Beam。
    expect(card?.querySelector(`[data-event-key="${beamA}"]`)).toBeNull();
    expect(card?.querySelector('.attention-beam')).toBeNull();
    // 总线中的 A 仍是 active，并可自然到期（200 + 1600 + 180ms）。
    expect(getAttentionBusState().active?.eventKey).toBe(beamA);
    await act(async () => { await sleep(2100); });
    expect(getAttentionBusState().active).toBeNull();
  });

  it('更新通知 A → 普通通知 B → 新更新通知 C：C 只显示 C 对应 Beam', async () => {
    render(<App />);
    mockPluginUpdate('2.0.0');
    await act(async () => { await checkForPluginUpdates(true); });

    const beamA = attentionPluginUpdateKey('mira-amaster', '2.0.0');
    expect(document.querySelector(`[data-event-key="${beamA}"]`)).not.toBeNull();
    await act(async () => { notifySuccess('中间普通通知'); });
    expect(document.querySelector(`[data-event-key="${beamA}"]`)).toBeNull();

    // 等 A 自然到期，让 C 能直接成为 active。
    await act(async () => { await sleep(2100); });
    invokeMock.mockImplementation((command: string) => {
      if (command === 'local_ai_updates_check') {
        return Promise.resolve([
          { component: 'bundle', currentVersion: '1.0.0', availableVersion: '1.2.0', updateAvailable: true },
        ]);
      }
      return Promise.reject(new Error('not mocked'));
    });
    await act(async () => { await checkForLocalAiUpdates(true); });

    const beamC = attentionLocalAiUpdateKey('bundle', '1.2.0');
    expect(getAttentionBusState().active?.eventKey).toBe(beamC);
    const card = document.querySelector('.app-notification');
    expect(card?.querySelector(`[data-event-key="${beamC}"]`)).not.toBeNull();
    expect(card?.querySelector(`[data-event-key="${beamA}"]`)).toBeNull();
  });

  it('同 eventKey 的 Beam 被会话去重拒绝时，当前通知不绑定旧 eventKey', async () => {
    render(<App />);
    mockPluginUpdate('2.0.0');
    await act(async () => { await checkForPluginUpdates(true); });

    const beamA = attentionPluginUpdateKey('mira-amaster', '2.0.0');
    expect(document.querySelector(`[data-event-key="${beamA}"]`)).not.toBeNull();
    // 再次发出完全相同的更新通知：announce 被会话去重拒绝。
    await act(async () => { await checkForPluginUpdates(true); });
    const card = document.querySelector('.app-notification');
    expect(card).not.toBeNull();
    expect(card?.querySelector(`[data-event-key="${beamA}"]`)).toBeNull();
    expect(card?.querySelector('.attention-beam')).toBeNull();
    // 旧 Beam 仍由总线自然管理，不被手动结束。
    expect(getAttentionBusState().active?.eventKey).toBe(beamA);
  });

  it('关闭通知同步解除 Beam eventKey 绑定', async () => {
    render(<App />);
    mockPluginUpdate('2.0.0');
    await act(async () => { await checkForPluginUpdates(true); });

    const beamA = attentionPluginUpdateKey('mira-amaster', '2.0.0');
    expect(document.querySelector(`[data-event-key="${beamA}"]`)).not.toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '关闭通知' }));
    expect(document.querySelector('.app-notification')).toBeNull();
    expect(document.querySelector('[data-event-key]')).toBeNull();
  });
});