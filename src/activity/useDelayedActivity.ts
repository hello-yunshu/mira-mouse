// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import {
  MIRA_ACTIVITY_MIN_VISIBLE_MS,
  MIRA_ACTIVITY_SHOW_DELAY_MS,
} from './activityCatalog';

/**
 * 延迟出现 + 最短可见时间。
 *
 * 快任务不会闪出 Orb；一旦出现，也不会在下一帧立即消失。
 *
 * exitHint 是“立即退出”提示：计数增长时，即使仍处于最短可见尾段也会立即
 * 隐藏 Orb（用于同 scope 完成事件出现前的仲裁）。被强制退出后，在任务真正
 * 结束（active 归 false）之前不再重新出现，避免“退出又回弹”的闪烁。
 * 延迟与退出由定时器回调驱动；显式 delayMs=0 时用 layout effect 在绘制前完成
 * 首帧切换，避免按钮先提交旧文案。
 */
export function useDelayedActivity(
  active: boolean,
  delayMs = MIRA_ACTIVITY_SHOW_DELAY_MS,
  minVisibleMs = MIRA_ACTIVITY_MIN_VISIBLE_MS,
  exitHint = 0,
): boolean {
  // delayMs=0 是调用方明确要求首帧显示；初次挂载时直接建立可见状态，避免
  // 先渲染业务提示、再等 effect + timer 补上 Orb 的空白帧。
  const [visible, setVisible] = useState(() => active && delayMs <= 0);
  const visibleSinceRef = useRef(0);
  const lastExitHintRef = useRef(0);
  const suppressedRef = useRef(false);

  // A zero delay is an explicit contract for the first active state,
  // including a transition from idle to active. A layout effect updates the
  // DOM before the browser paints, so a button never visibly commits its old
  // label as an intermediate frame.
  useLayoutEffect(() => {
    if (active && delayMs <= 0 && !visible && !suppressedRef.current && exitHint <= lastExitHintRef.current) {
      visibleSinceRef.current = performance.now();
      setVisible(true);
    }
  }, [active, delayMs, visible, exitHint]);

  useEffect(() => {
    let timer = 0;

    if (exitHint > lastExitHintRef.current) {
      lastExitHintRef.current = exitHint;
      suppressedRef.current = true;
      timer = window.setTimeout(() => setVisible(false), 0);
    } else if (active) {
      if (!visible && !suppressedRef.current) {
        timer = window.setTimeout(() => {
          visibleSinceRef.current = performance.now();
          setVisible(true);
        }, Math.max(0, delayMs));
      }
    } else {
      // 任务结束：解除强制退出抑制，允许下一个任务正常出现。
      suppressedRef.current = false;
      if (visible) {
        const elapsed = performance.now() - visibleSinceRef.current;
        timer = window.setTimeout(
          () => setVisible(false),
          Math.max(0, minVisibleMs - elapsed),
        );
      }
    }

    return () => window.clearTimeout(timer);
  }, [active, delayMs, minVisibleMs, visible, exitHint]);

  return visible;
}
