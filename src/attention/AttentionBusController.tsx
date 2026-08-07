// SPDX-License-Identifier: AGPL-3.0-or-later
// AttentionBusController —— 全局请求生命周期的唯一管理者，挂在 App 根部，
// 不受视图 / 设备 / 通知 / 页面挂载影响。
//
// 职责：
// - 订阅总线状态变化（不渲染任何 UI，返回 null）；
// - 为当前 active 建立唯一的结束计时器；
// - 计时到期且 active 未变化时推进总线的 pending 队列；
// - 无论 active 是否匹配某个已挂载的 Surface，都会结束（杜绝无 Scope 锁死）。

import { useEffect, useReducer } from 'react';
import {
  finishActiveAttentionRequest,
  getAttentionBusState,
  onAttentionBusStateChange,
} from './attentionCore';
import {
  attentionRequestTotalMs,
  prefersReducedAttentionMotion,
} from './attentionTiming';

export function AttentionBusController(): null {
  const [, refresh] = useReducer((value: number) => value + 1, 0);

  // 订阅总线：每次 active/pending 变化都触发一次刷新，让下方 effect 重新建立
  // 与当前 active 对应的计时器（active 变化时旧计时器被清理，不会误伤新请求）。
  useEffect(() => onAttentionBusStateChange(refresh), []);

  const active = getAttentionBusState().active;

  useEffect(() => {
    if (!active) return;

    const eventKey = active.eventKey;
    const totalMs = attentionRequestTotalMs(active, prefersReducedAttentionMotion());

    const timer = window.setTimeout(() => {
      const current = getAttentionBusState().active;
      if (current?.eventKey === eventKey) {
        finishActiveAttentionRequest();
      }
    }, totalMs);

    return () => {
      window.clearTimeout(timer);
    };
  }, [active]);

  return null;
}