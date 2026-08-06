// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from 'react';
import {
  MIRA_ACTIVITY_MIN_VISIBLE_MS,
  MIRA_ACTIVITY_SHOW_DELAY_MS,
} from './activityCatalog';

/**
 * 延迟出现 + 最短可见时间。
 *
 * 快任务不会闪出 Orb；一旦出现，也不会在下一帧立即消失。
 */
export function useDelayedActivity(
  active: boolean,
  delayMs = MIRA_ACTIVITY_SHOW_DELAY_MS,
  minVisibleMs = MIRA_ACTIVITY_MIN_VISIBLE_MS,
): boolean {
  const [visible, setVisible] = useState(false);
  const visibleSinceRef = useRef(0);

  useEffect(() => {
    let timer = 0;

    if (active) {
      if (!visible) {
        timer = window.setTimeout(() => {
          visibleSinceRef.current = performance.now();
          setVisible(true);
        }, Math.max(0, delayMs));
      }
    } else if (visible) {
      const elapsed = performance.now() - visibleSinceRef.current;
      timer = window.setTimeout(
        () => setVisible(false),
        Math.max(0, minVisibleMs - elapsed),
      );
    }

    return () => window.clearTimeout(timer);
  }, [active, delayMs, minVisibleMs, visible]);

  return visible;
}
