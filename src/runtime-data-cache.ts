// SPDX-License-Identifier: AGPL-3.0-or-later
import { invoke } from '@tauri-apps/api/core';
import type { AboutInfo, AppSettings, LocalAiStatus } from './types';

type RuntimeResource<T> = {
  value?: T;
  pending?: Promise<T>;
};

const aboutInfoResource: RuntimeResource<AboutInfo> = {};
const appSettingsResource: RuntimeResource<AppSettings> = {};
const localAiStatusResource: RuntimeResource<LocalAiStatus> = {};

function loadResource<T>(
  resource: RuntimeResource<T>,
  command: string,
  refresh = false,
): Promise<T> {
  if (!refresh && resource.value !== undefined) return Promise.resolve(resource.value);
  if (resource.pending) return resource.pending;

  const request = invoke<T>(command)
    .then((value) => {
      resource.value = value;
      return value;
    })
    .finally(() => {
      if (resource.pending === request) resource.pending = undefined;
    });
  resource.pending = request;
  return request;
}

export function peekAboutInfo(): AboutInfo | undefined {
  return aboutInfoResource.value;
}

export function loadAboutInfo(options: { refresh?: boolean } = {}): Promise<AboutInfo> {
  return loadResource(aboutInfoResource, 'about_info', options.refresh);
}

export function invalidateAboutInfo(): void {
  aboutInfoResource.value = undefined;
}

export function peekAppSettings(): AppSettings | undefined {
  return appSettingsResource.value;
}

export function loadAppSettings(options: { refresh?: boolean } = {}): Promise<AppSettings> {
  return loadResource(appSettingsResource, 'settings_get', options.refresh);
}

export function storeAppSettings(settings: AppSettings): void {
  appSettingsResource.value = settings;
}

export function peekLocalAiStatus(): LocalAiStatus | undefined {
  return localAiStatusResource.value;
}

export function loadLocalAiStatus(options: { refresh?: boolean } = {}): Promise<LocalAiStatus> {
  return loadResource(localAiStatusResource, 'local_ai_status', options.refresh);
}

export function storeLocalAiStatus(status: LocalAiStatus): void {
  localAiStatusResource.value = status;
}

export function resetRuntimeDataCacheForTests(): void {
  for (const resource of [aboutInfoResource, appSettingsResource, localAiStatusResource]) {
    resource.value = undefined;
    resource.pending = undefined;
  }
}

// 测试环境需要在每个用例后清空模块级快照；通过轻量全局钩子避免
// test-setup 提前 import 本模块，从而绕过各测试文件的 Tauri invoke mock。
if (import.meta.env.MODE === 'test') {
  (globalThis as typeof globalThis & {
    __MIRA_RESET_RUNTIME_DATA_CACHE__?: () => void;
  }).__MIRA_RESET_RUNTIME_DATA_CACHE__ = resetRuntimeDataCacheForTests;
}
