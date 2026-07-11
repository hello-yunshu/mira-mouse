// SPDX-License-Identifier: AGPL-3.0-or-later
export interface AutomaticUpdateSchedulerOptions {
  intervalMs: number;
  /** Returns whether a real update check was started, so only those runs reset the due time. */
  run: () => Promise<boolean>;
}

export interface AutomaticUpdateScheduler {
  start: (enabled: boolean) => Promise<void>;
  stop: () => void;
}

/**
 * Shared lifecycle for application and plugin update checks. The updater-specific
 * caller still owns its state machine, notifications, downloads, and validation.
 */
export function createAutomaticUpdateScheduler({ intervalMs, run }: AutomaticUpdateSchedulerOptions): AutomaticUpdateScheduler {
  let started = false;
  let timer: ReturnType<typeof window.setInterval> | undefined;
  let lastRunAt: number | undefined;

  async function trigger(): Promise<void> {
    if (await run()) lastRunAt = Date.now();
  }

  function triggerIfDue(): void {
    if (lastRunAt === undefined || Date.now() - lastRunAt >= intervalMs) {
      void trigger().catch(() => {});
    }
  }

  function handleVisibilityChange(): void {
    if (document.visibilityState === 'visible') triggerIfDue();
  }

  function ensureSchedule(): void {
    if (timer !== undefined || typeof window === 'undefined') return;
    timer = window.setInterval(triggerIfDue, intervalMs);
    window.addEventListener('online', triggerIfDue);
    window.addEventListener('focus', triggerIfDue);
    document.addEventListener('visibilitychange', handleVisibilityChange);
  }

  function stop(): void {
    if (typeof window === 'undefined') return;
    if (timer !== undefined) {
      window.clearInterval(timer);
      timer = undefined;
    }
    window.removeEventListener('online', triggerIfDue);
    window.removeEventListener('focus', triggerIfDue);
    document.removeEventListener('visibilitychange', handleVisibilityChange);
    started = false;
    lastRunAt = undefined;
  }

  async function start(enabled: boolean): Promise<void> {
    if (!enabled) {
      stop();
      return;
    }
    ensureSchedule();
    if (started) return;
    started = true;
    await trigger();
  }

  return { start, stop };
}
