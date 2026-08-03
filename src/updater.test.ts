// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  invoke: vi.fn(),
  downloadAndInstall: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-updater', () => ({ check: mocks.check }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import {
  APP_UPDATE_CHECK_INTERVAL_MS,
  appUpdateState,
  checkForAppUpdate,
  installAppUpdate,
  relaunchAfterUpdate,
  startAutomaticAppUpdateCheck,
  stopAutomaticAppUpdateCheck,
  recordUpdateReminderDismissed,
  recordUpdateReminderIgnored,
  remindInstalledUpdateOnShown,
} from './updater';
import { onAppNotification, type AppNotification } from './notify';

describe('application updater', () => {
  beforeEach(() => {
    mocks.check.mockReset();
    mocks.invoke.mockReset();
    mocks.downloadAndInstall.mockReset();
    mocks.invoke.mockResolvedValue(undefined);
  });

  afterEach(() => {
    stopAutomaticAppUpdateCheck();
    vi.useRealTimers();
  });

  it('sends a system notification during automatic checks when an update is available', async () => {
    mocks.check.mockResolvedValue({
      version: '0.3.0',
      body: 'Release notes',
      date: '2026-06-23T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });
    await checkForAppUpdate(true);
    expect(mocks.invoke).toHaveBeenCalledWith('show_update_notification', {
      title: '发现新版本',
      body: 'v0.3.0 已可用，可在「关于」页查看并安装。',
      action: 'about-update',
    });
  });

  it('keeps the checked update, reports progress, installs, and relaunches', async () => {
    mocks.downloadAndInstall.mockImplementation(async (onEvent) => {
      onEvent({ event: 'Started', data: { contentLength: 100 } });
      onEvent({ event: 'Progress', data: { chunkLength: 40 } });
      onEvent({ event: 'Progress', data: { chunkLength: 60 } });
      onEvent({ event: 'Finished' });
    });
    mocks.check.mockResolvedValue({
      version: '0.2.0',
      body: 'Release notes',
      date: '2026-06-23T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });
    await checkForAppUpdate();
    expect(appUpdateState()).toMatchObject({ phase: 'available', version: '0.2.0' });
    await installAppUpdate();
    expect(mocks.check).toHaveBeenCalledTimes(1);
    expect(appUpdateState()).toMatchObject({ phase: 'installed', downloadedBytes: 100, totalBytes: 100 });
    await relaunchAfterUpdate();
    expect(mocks.invoke).toHaveBeenCalledWith('relaunch_app');
  });

  it('does not send a native update notification when a manual download fails', async () => {
    mocks.downloadAndInstall.mockRejectedValue(new Error('network down'));
    mocks.check.mockResolvedValue({
      version: '0.3.9',
      body: 'Release notes',
      date: '2026-07-01T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });
    await checkForAppUpdate();
    await expect(installAppUpdate()).rejects.toThrow('network down');
    expect(appUpdateState()).toMatchObject({ phase: 'error', error: 'network down' });
    expect(mocks.invoke).not.toHaveBeenCalledWith('show_update_notification', expect.anything());
  });

  it('continues automatic checks while the app stays open in the background', async () => {
    vi.useFakeTimers();
    mocks.check.mockResolvedValue(null);

    await startAutomaticAppUpdateCheck(true);
    expect(mocks.check).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(APP_UPDATE_CHECK_INTERVAL_MS - 1);
    expect(mocks.check).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(1);
    expect(mocks.check).toHaveBeenCalledTimes(2);
  });

  it('skips the update-found notification when automatic install is enabled', async () => {
    mocks.downloadAndInstall.mockImplementation(async (onEvent) => {
      onEvent({ event: 'Started', data: { contentLength: 100 } });
      onEvent({ event: 'Finished' });
    });
    mocks.check.mockResolvedValue({
      version: '0.3.0',
      body: 'Release notes',
      date: '2026-06-23T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });
    await startAutomaticAppUpdateCheck(true, true);
    expect(mocks.invoke).not.toHaveBeenCalledWith('show_update_notification', expect.objectContaining({ title: '发现新版本' }));
    expect(mocks.invoke).toHaveBeenCalledWith('show_update_notification', expect.objectContaining({ title: '更新就绪', action: 'about-update' }));
    expect(appUpdateState()).toMatchObject({ phase: 'installed' });
  });

  it('reminds to relaunch when the main window is shown after an update is installed', async () => {
    mocks.downloadAndInstall.mockImplementation(async (onEvent) => {
      onEvent({ event: 'Started', data: { contentLength: 100 } });
      onEvent({ event: 'Finished' });
    });
    mocks.check.mockResolvedValue({
      version: '0.2.0',
      body: 'Release notes',
      date: '2026-06-23T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });

    const notifications: AppNotification[] = [];
    const unsubscribe = onAppNotification((notification) => notifications.push(notification));

    await checkForAppUpdate();
    await installAppUpdate();
    expect(appUpdateState().phase).toBe('installed');
    // 安装完成时发送一次应用内通知
    expect(notifications.filter((n) => n.action === 'relaunch')).toHaveLength(1);

    // 模拟主窗口从托盘恢复
    remindInstalledUpdateOnShown();
    expect(notifications.filter((n) => n.action === 'relaunch')).toHaveLength(2);

    // 再次恢复，第三次提醒
    remindInstalledUpdateOnShown();
    expect(notifications.filter((n) => n.action === 'relaunch')).toHaveLength(3);

    unsubscribe();
  });

  it('stops reminding after the user dismisses the notification twice', async () => {
    mocks.downloadAndInstall.mockImplementation(async (onEvent) => {
      onEvent({ event: 'Started', data: { contentLength: 100 } });
      onEvent({ event: 'Finished' });
    });
    mocks.check.mockResolvedValue({
      version: '0.2.0',
      body: 'Release notes',
      date: '2026-06-23T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });

    const notifications: AppNotification[] = [];
    const unsubscribe = onAppNotification((notification) => notifications.push(notification));

    await checkForAppUpdate();
    await installAppUpdate();
    expect(notifications.filter((n) => n.action === 'relaunch')).toHaveLength(1);

    // 用户主动关闭 2 次，达到阈值
    recordUpdateReminderDismissed();
    recordUpdateReminderDismissed();

    // 主窗口恢复不再触发提醒
    remindInstalledUpdateOnShown();
    expect(notifications.filter((n) => n.action === 'relaunch')).toHaveLength(1);

    unsubscribe();
  });

  it('stops reminding after the user ignores the notification three times', async () => {
    mocks.downloadAndInstall.mockImplementation(async (onEvent) => {
      onEvent({ event: 'Started', data: { contentLength: 100 } });
      onEvent({ event: 'Finished' });
    });
    mocks.check.mockResolvedValue({
      version: '0.2.0',
      body: 'Release notes',
      date: '2026-06-23T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });

    const notifications: AppNotification[] = [];
    const unsubscribe = onAppNotification((notification) => notifications.push(notification));

    await checkForAppUpdate();
    await installAppUpdate();
    expect(notifications.filter((n) => n.action === 'relaunch')).toHaveLength(1);

    // 超时忽视 3 次，达到阈值
    recordUpdateReminderIgnored();
    recordUpdateReminderIgnored();
    recordUpdateReminderIgnored();

    remindInstalledUpdateOnShown();
    expect(notifications.filter((n) => n.action === 'relaunch')).toHaveLength(1);

    unsubscribe();
  });

  it('resets dismiss/ignore counters on a new install cycle', async () => {
    mocks.downloadAndInstall.mockImplementation(async (onEvent) => {
      onEvent({ event: 'Started', data: { contentLength: 100 } });
      onEvent({ event: 'Finished' });
    });
    mocks.check.mockResolvedValue({
      version: '0.2.0',
      body: 'Release notes',
      date: '2026-06-23T00:00:00Z',
      downloadAndInstall: mocks.downloadAndInstall,
      close: vi.fn().mockResolvedValue(undefined),
    });

    const notifications: AppNotification[] = [];
    const unsubscribe = onAppNotification((notification) => notifications.push(notification));

    await checkForAppUpdate();
    await installAppUpdate();
    // 关闭 2 次达到阈值
    recordUpdateReminderDismissed();
    recordUpdateReminderDismissed();
    remindInstalledUpdateOnShown();
    expect(notifications.filter((n) => n.action === 'relaunch')).toHaveLength(1);

    // 新的安装周期重置计数
    await installAppUpdate();
    expect(notifications.filter((n) => n.action === 'relaunch')).toHaveLength(2);
    // 窗口恢复再次触发提醒
    remindInstalledUpdateOnShown();
    expect(notifications.filter((n) => n.action === 'relaunch')).toHaveLength(3);

    unsubscribe();
  });

  it('does not remind when no update is installed', async () => {
    mocks.check.mockResolvedValue(null);
    await checkForAppUpdate();
    expect(appUpdateState().phase).toBe('up-to-date');

    const notifications: AppNotification[] = [];
    const unsubscribe = onAppNotification((notification) => notifications.push(notification));
    remindInstalledUpdateOnShown();
    expect(notifications.filter((n) => n.action === 'relaunch')).toHaveLength(0);
    unsubscribe();
  });
});
