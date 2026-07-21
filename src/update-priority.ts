// SPDX-License-Identifier: AGPL-3.0-or-later
// 更新通知优先级协调层：主程序更新通知优先级最高。
// 当主程序有可用更新或正在下载/安装时，压制其他组件（插件、AI 引擎）的
// 系统级更新通知，避免通知栏同时出现多条更新提示。应用内 Toast 仍然发出。
//
// 协调层只读 `src/updater.ts` 的状态，不修改它；其他模块按需查询即可。
import { appUpdateState, onAppUpdateState, type AppUpdateState } from './updater';

type Listener = (suppressed: boolean) => void;

const target = new EventTarget();
let lastSuppressed = false;

function isAppUpdateActive(state: AppUpdateState): boolean {
  // 'installed' 也算激活：刚装完等待重启，用户的注意力应当集中在主程序上。
  return (
    state.phase === 'available'
    || state.phase === 'downloading'
    || state.phase === 'installed'
  );
}

function publish(nextSuppressed: boolean): void {
  if (nextSuppressed === lastSuppressed) return;
  lastSuppressed = nextSuppressed;
  target.dispatchEvent(new CustomEvent<boolean>('change', { detail: nextSuppressed }));
}

export function isComponentUpdateNotificationSuppressed(): boolean {
  return lastSuppressed;
}

export function onComponentUpdateSuppression(listener: Listener): () => void {
  const handler = (event: Event) => listener((event as CustomEvent<boolean>).detail);
  target.addEventListener('change', handler);
  listener(lastSuppressed);
  return () => target.removeEventListener('change', handler);
}

let initialized = false;

/**
 * 订阅主程序更新状态，自动维护 `isComponentUpdateNotificationSuppressed()`。
 * 由 App.tsx 在启动时调用一次即可；多次调用是幂等的。
 */
export function initUpdatePriorityCoordinator(): void {
  if (initialized) return;
  initialized = true;
  onAppUpdateState((state) => publish(isAppUpdateActive(state)));
  publish(isAppUpdateActive(appUpdateState()));
}
