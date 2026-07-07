// SPDX-License-Identifier: AGPL-3.0-or-later
export interface AppNotification {
  title: string;
  body?: string;
  kind: 'error' | 'info' | 'success';
  action?: 'about-update' | 'settings-plugin-update' | 'battery-usage' | 'relaunch';
}

const notificationTarget = new EventTarget();

export function onAppNotification(listener: (notification: AppNotification) => void): () => void {
  const handler = (event: Event) => listener((event as CustomEvent<AppNotification>).detail);
  notificationTarget.addEventListener('notification', handler);
  return () => notificationTarget.removeEventListener('notification', handler);
}

/** Report foreground operation failures inside the app; background alerts use native notifications. */
export function notifyError(title: string, body?: string): void {
  notificationTarget.dispatchEvent(new CustomEvent<AppNotification>('notification', {
    detail: { title, body, kind: 'error' },
  }));
  console.error(title, body ?? '');
}

export function notifyInfo(title: string, body?: string, action?: AppNotification['action']): void {
  notificationTarget.dispatchEvent(new CustomEvent<AppNotification>('notification', {
    detail: { title, body, kind: 'info', action },
  }));
}

export function notifySuccess(title: string, body?: string): void {
  notificationTarget.dispatchEvent(new CustomEvent<AppNotification>('notification', {
    detail: { title, body, kind: 'success' },
  }));
}
