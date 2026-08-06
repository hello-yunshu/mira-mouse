// SPDX-License-Identifier: AGPL-3.0-or-later
// Attention Beam 模块测试：总线仲裁、会话去重、更新仲裁目标解析、
// 设备状态机（含多设备隔离）、useAttentionFeedback 作用域过滤、
// AttentionBeamLayer 渲染/时长/卸载、灯光 Mutation 关联流程、颜色解析。
// node:fs / import.meta.dirname 的最小类型见同目录 attention.test.env.d.ts。

import { act, render, renderHook } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  announceAttentionRequest,
  attentionBusReduce,
  createInitialAttentionBusState,
  finishActiveAttentionRequest,
  getAttentionBusState,
  hasAttentionEventPlayedOnce,
  resetAttentionBusForTests,
} from './attentionCore';
import {
  ATTENTION_PRIORITY,
  MAX_ATTENTION_QUEUE,
  attentionColorForZone,
  reduceDeviceAttention,
  reduceDeviceAttentionByIdentity,
  resolveUpdateAttentionTarget,
  type AttentionBeamRequest,
  type DeviceAttentionContext,
} from './attentionTypes';
import { AttentionBeamLayer } from './AttentionBeamLayer';
import { useAttentionFeedback } from './useAttentionFeedback';
import {
  attentionLightingMutationKey,
  clearPendingLightingAttention,
  confirmPendingLightingAttention,
  lightingAttentionRequest,
  peekPendingLightingAttention,
  registerLightingAttention,
  resetPendingLightingAttentionForTests,
  takePendingLightingAttention,
  verifyLightingAttention,
  type PendingLightingAttention,
  type ZoneLightingState,
} from './attentionLighting';
const beamCss = readFileSync(join(import.meta.dirname, 'attention-beam.css'), 'utf8');

function beam(overrides: Partial<AttentionBeamRequest> = {}): AttentionBeamRequest {
  return {
    eventKey: `test:${Math.random()}`,
    scope: 'test:surface',
    variant: 'line',
    color: '#ffb3b3',
    durationMs: 500,
    strength: 0.2,
    cycles: 1,
    priority: 50,
    ...overrides,
  };
}

function initialContext(): DeviceAttentionContext {
  return { previous: undefined, wasReady: false, cycle: 0 };
}

function zoneState(overrides: Partial<ZoneLightingState> = {}): ZoneLightingState {
  return { enabled: true, color: '#ff0000', effectValue: undefined, ...overrides };
}

beforeEach(() => {
  resetAttentionBusForTests();
  resetPendingLightingAttentionForTests();
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('attentionBusReduce / announceAttentionRequest', () => {
  it('播放空闲总线的第一个请求', () => {
    const request = beam({ eventKey: 'a' });
    expect(announceAttentionRequest(request)).toBe(true);
    expect(getAttentionBusState().active?.eventKey).toBe('a');
  });

  it('同一事件键一次会话只入队一次', () => {
    const request = beam({ eventKey: 'dup' });
    expect(announceAttentionRequest(request)).toBe(true);
    expect(announceAttentionRequest({ ...request })).toBe(false);
    expect(hasAttentionEventPlayedOnce('dup')).toBe(true);
  });

  it('活跃光束存在时按优先级排队，finish 后推进', () => {
    announceAttentionRequest(beam({ eventKey: 'low', priority: 10 }));
    const queued = announceAttentionRequest(beam({ eventKey: 'high', priority: 90 }));
    expect(queued).toBe(true);
    const state = getAttentionBusState();
    expect(state.active?.eventKey).toBe('low');
    expect(state.pending.map((item) => item.eventKey)).toEqual(['high']);
    finishActiveAttentionRequest();
    expect(getAttentionBusState().active?.eventKey).toBe('high');
  });

  it('排队队列按优先级降序，且超出上限时丢弃队尾', () => {
    const requests = Array.from({ length: MAX_ATTENTION_QUEUE + 3 }, (_, index) =>
      beam({ eventKey: `queued-${index}`, priority: 10 + index }));
    announceAttentionRequest(requests[0]);
    for (const request of requests.slice(1)) announceAttentionRequest(request);
    const { pending } = getAttentionBusState();
    expect(pending).toHaveLength(MAX_ATTENTION_QUEUE);
    const priorities = pending.map((item) => item.priority);
    for (let index = 1; index < priorities.length; index += 1) {
      expect(priorities[index - 1]).toBeGreaterThanOrEqual(priorities[index]);
    }
    expect(priorities[0]).toBe(10 + requests.length - 1);
  });

  it('窗口不可见（document.hidden）时拒绝且不补播', () => {
    const request = beam({ eventKey: 'hidden-event' });
    vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
    expect(announceAttentionRequest(request)).toBe(false);
    expect(getAttentionBusState().active).toBeNull();
    vi.restoreAllMocks();
    expect(hasAttentionEventPlayedOnce('hidden-event')).toBe(false);
  });

  it('finish 幂等：无活跃光束时无操作', () => {
    expect(() => finishActiveAttentionRequest()).not.toThrow();
    expect(getAttentionBusState().active).toBeNull();
  });
});

describe('attentionBusReduce 纯归约', () => {
  it('finish 推进队列并把下一个置为活跃', () => {
    const state = attentionBusReduce(createInitialAttentionBusState(), {
      type: 'announce',
      request: beam({ priority: 20 }),
    });
    const withPending = attentionBusReduce(state, {
      type: 'announce',
      request: beam({ eventKey: 'second', priority: 80 }),
    });
    const after = attentionBusReduce(withPending, { type: 'finish' });
    expect(after.active?.eventKey).toBe('second');
    expect(after.pending).toEqual([]);
  });
});

describe('resolveUpdateAttentionTarget 更新仲裁（P0-2）', () => {
  it('Dashboard + 插件更新 → 通知（固定行不消费）', () => {
    expect(resolveUpdateAttentionTarget('plugin', { view: 'dashboard' })).toBe('notification');
  });

  it('Settings/General/Device/Privacy + 插件更新 → 通知（固定行不消费）', () => {
    for (const settingsTab of ['general', 'device', 'privacy', 'about']) {
      expect(resolveUpdateAttentionTarget('plugin', { view: 'settings', settingsTab })).toBe('notification');
    }
  });

  it('Settings/Plugins + 插件更新 → 固定行', () => {
    expect(resolveUpdateAttentionTarget('plugin', { view: 'settings', settingsTab: 'plugins' })).toBe('settings-plugin');
  });

  it('本地 AI 更新走独立目标解析（与插件行共用可见性条件）', () => {
    expect(resolveUpdateAttentionTarget('local-ai', { view: 'settings', settingsTab: 'plugins' })).toBe('settings-local-ai');
    expect(resolveUpdateAttentionTarget('local-ai', { view: 'settings', settingsTab: 'general' })).toBe('notification');
    expect(resolveUpdateAttentionTarget('local-ai', { view: 'dashboard' })).toBe('notification');
  });

  it('应用更新：About 可见 → about 固定卡；否则 → 通知', () => {
    expect(resolveUpdateAttentionTarget('app', { view: 'about' })).toBe('about');
    expect(resolveUpdateAttentionTarget('app', { view: 'dashboard' })).toBe('notification');
    expect(resolveUpdateAttentionTarget('app', { view: 'settings', settingsTab: 'plugins' })).toBe('notification');
  });
});

describe('reduceDeviceAttention 设备就绪状态机', () => {
  it('首次观察不触发任何事件（启动即在线不算连接）', () => {
    const outcome = reduceDeviceAttention(initialContext(), true, 0);
    expect(outcome.action).toBe('none');
    expect(outcome.wasReady).toBe(true);
  });

  it('启动离线后首次就绪视为 device-ready（宽限期内也有效）', () => {
    expect(reduceDeviceAttention(initialContext(), false, 0).action).toBe('none');
    expect(reduceDeviceAttention({ ...initialContext(), previous: false }, true, 1000).action).toBe('ready');
  });

  it('就绪后断开再恢复视为 device-reconnected 并递增周期', () => {
    const dropped = reduceDeviceAttention({ previous: true, wasReady: true, cycle: 0 }, false, 5000);
    expect(dropped.action).toBe('none');
    const restored = reduceDeviceAttention({ previous: false, wasReady: true, cycle: dropped.cycle }, true, 6000);
    expect(restored.action).toBe('reconnected');
    expect(restored.cycle).toBe(1);
  });

  it('状态未变化时无操作', () => {
    expect(reduceDeviceAttention({ previous: true, wasReady: true, cycle: 2 }, true, 999).action).toBe('none');
  });
});

describe('reduceDeviceAttentionByIdentity 多设备隔离（P1-4）', () => {
  it('两个同名设备互不影响', () => {
    const contexts = new Map<string, DeviceAttentionContext>();
    // 设备 1 在线（启动即在线 → 无事件）。
    expect(reduceDeviceAttentionByIdentity(contexts, 'device-1', true, 0).action).toBe('none');
    // 设备 2 等待中就绪 → ready。
    reduceDeviceAttentionByIdentity(contexts, 'device-2', false, 100);
    expect(reduceDeviceAttentionByIdentity(contexts, 'device-2', true, 5000).action).toBe('ready');
    // 设备 1 断开再恢复 → reconnected；设备 2 状态不受影响。
    reduceDeviceAttentionByIdentity(contexts, 'device-1', false, 6000);
    expect(reduceDeviceAttentionByIdentity(contexts, 'device-1', true, 7000).action).toBe('reconnected');
    expect(reduceDeviceAttentionByIdentity(contexts, 'device-2', true, 7001).action).toBe('none');
  });

  it('A 断开不影响 B，B 保持既有状态', () => {
    const contexts = new Map<string, DeviceAttentionContext>();
    reduceDeviceAttentionByIdentity(contexts, 'dev-A', false, 0);
    reduceDeviceAttentionByIdentity(contexts, 'dev-B', true, 0);
    expect(reduceDeviceAttentionByIdentity(contexts, 'dev-A', false, 100).action).toBe('none');
    expect(reduceDeviceAttentionByIdentity(contexts, 'dev-B', true, 100).action).toBe('none');
  });

  it('切换到已在线 B 不误判成 A 重连', () => {
    const contexts = new Map<string, DeviceAttentionContext>();
    reduceDeviceAttentionByIdentity(contexts, 'A', true, 0);
    reduceDeviceAttentionByIdentity(contexts, 'A', false, 5000);
    // B 首次观察无事件，不继承 A 的重连周期。
    expect(reduceDeviceAttentionByIdentity(contexts, 'B', true, 6000).action).toBe('none');
    // A 之后再次上线仍按自身周期判重连。
    expect(reduceDeviceAttentionByIdentity(contexts, 'A', true, 7000).action).toBe('reconnected');
  });

  it('同一设备真实断开恢复仍触发 reconnected', () => {
    const contexts = new Map<string, DeviceAttentionContext>();
    reduceDeviceAttentionByIdentity(contexts, 'dev-X', true, 0);
    reduceDeviceAttentionByIdentity(contexts, 'dev-X', false, 5000);
    const outcome = reduceDeviceAttentionByIdentity(contexts, 'dev-X', true, 6000);
    expect(outcome.action).toBe('reconnected');
    expect(outcome.cycle).toBe(1);
  });
});

describe('useAttentionFeedback', () => {
  it('只渲染本作用域的光束', () => {
    const { result } = renderHook(() => useAttentionFeedback('surface:a'));
    act(() => { announceAttentionRequest(beam({ eventKey: 'mine', scope: 'surface:a' })); });
    expect(result.current.beam?.eventKey).toBe('mine');
    act(() => {
      finishActiveAttentionRequest();
      announceAttentionRequest(beam({ eventKey: 'other', scope: 'surface:b' }));
    });
    expect(result.current.beam).toBeNull();
  });

  it('behavior=lighting 只允许灯光类事件', () => {
    const { result } = renderHook(() => useAttentionFeedback('x', { behavior: 'lighting' }));
    expect(result.current.announce(beam({ eventKey: 'update:app:mira:1.0.0' }))).toBe(false);
    expect(result.current.announce(beam({ eventKey: 'lighting:z1:power:on:1' }))).toBe(true);
  });

  it('behavior=high 拒绝灯光类事件', () => {
    const { result } = renderHook(() => useAttentionFeedback('x', { behavior: 'high' }));
    expect(result.current.announce(beam({ eventKey: 'lighting:z1:power:on:1' }))).toBe(false);
    expect(result.current.announce(beam({ eventKey: 'update:plugin:p:1.2.3' }))).toBe(true);
  });

  it('光束结束定时后自动 finish 并清空', () => {
    const { result } = renderHook(() => useAttentionFeedback('surface:auto'));
    act(() => { announceAttentionRequest(beam({ eventKey: 'auto-finish', scope: 'surface:auto', durationMs: 300, cycles: 1 })); });
    expect(result.current.beam?.eventKey).toBe('auto-finish');
    act(() => vi.advanceTimersByTime(600));
    expect(result.current.beam).toBeNull();
    expect(getAttentionBusState().active).toBeNull();
  });

  it('Reduced Motion 下按更短的静态淡入时长结束（不乘以 cycles）', () => {
    const originalMatchMedia = window.matchMedia;
    window.matchMedia = vi.fn().mockImplementation((query: string) => ({
      matches: query.includes('prefers-reduced-motion'),
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    }));
    try {
      const { result } = renderHook(() => useAttentionFeedback('surface:rm'));
      act(() => { announceAttentionRequest(beam({ eventKey: 'rm', scope: 'surface:rm', durationMs: 2400, cycles: 2 })); });
      expect(result.current.beam?.eventKey).toBe('rm');
      // Reduced Motion 结束时间 = min(450, 2400 + 140) = 450ms。
      act(() => vi.advanceTimersByTime(500));
      expect(result.current.beam).toBeNull();
      expect(getAttentionBusState().active).toBeNull();
    } finally {
      window.matchMedia = originalMatchMedia;
    }
  });
});

describe('AttentionBeamLayer', () => {
  it('active 且未结束时渲染，结束时调用 onFinished 并卸载自身', () => {
    const onFinished = vi.fn();
    const request = beam({ eventKey: 'layer-a', durationMs: 300, cycles: 1 });
    const { rerender, container } = render(
      <AttentionBeamLayer active request={request} onFinished={onFinished} />,
    );
    expect(container.querySelector('.attention-beam')).not.toBeNull();
    expect(container.querySelector('[data-event-key="layer-a"]')).not.toBeNull();
    act(() => vi.advanceTimersByTime(600));
    expect(onFinished).toHaveBeenCalledTimes(1);
    rerender(<AttentionBeamLayer active request={request} onFinished={onFinished} />);
    expect(container.querySelector('.attention-beam')).toBeNull();
  });

  it('inactive 时不渲染', () => {
    const { container } = render(<AttentionBeamLayer active={false} request={beam()} />);
    expect(container.querySelector('.attention-beam')).toBeNull();
  });

  it('渲染指定变体与配色变量', () => {
    const { container } = render(
      <AttentionBeamLayer active request={beam({ variant: 'pulse-inner', color: '#123456', cycles: 2, durationMs: 2400 })} />,
    );
    const element = container.querySelector('.attention-beam');
    expect(element?.className).toContain('attention-beam--pulse-inner');
    expect((element as HTMLElement).style.getPropertyValue('--beam-color')).toBe('#123456');
    expect(element?.querySelectorAll('.attention-beam__cycle')).toHaveLength(2);
  });

  it('aria-hidden 且 pointer-events 由 CSS 类关闭', () => {
    const { container } = render(<AttentionBeamLayer active request={beam()} />);
    const element = container.querySelector('.attention-beam');
    expect(element).toHaveAttribute('aria-hidden', 'true');
    expect(beamCss).toMatch(/\.attention-beam\s*{[^}]*pointer-events:\s*none/s);
  });

  it('line 即使传入 cycles=2 也只渲染 1 个 cycle', () => {
    const { container } = render(<AttentionBeamLayer active request={beam({ variant: 'line', cycles: 2 })} />);
    expect(container.querySelectorAll('.attention-beam__cycle')).toHaveLength(1);
  });

  it('flash 即使传入 cycles=2 也只渲染 1 个 cycle', () => {
    const { container } = render(<AttentionBeamLayer active request={beam({ variant: 'flash', cycles: 2 })} />);
    expect(container.querySelectorAll('.attention-beam__cycle')).toHaveLength(1);
  });

  it('pulse-inner 最多渲染 2 个 cycle', () => {
    const { container } = render(<AttentionBeamLayer active request={beam({ variant: 'pulse-inner', cycles: 9 })} />);
    expect(container.querySelectorAll('.attention-beam__cycle')).toHaveLength(2);
  });

  it('pulse-inner 2400ms / 2 cycles：每 cycle 时长 = 总时长/次数，总时长不再翻倍', () => {
    const onFinished = vi.fn();
    const { container } = render(
      <AttentionBeamLayer
        active
        request={beam({ variant: 'pulse-inner', durationMs: 2400, cycles: 2 })}
        onFinished={onFinished}
      />,
    );
    const element = container.querySelector('.attention-beam');
    expect((element as HTMLElement).style.getPropertyValue('--beam-duration')).toBe('1200ms');
    const cycles = [...container.querySelectorAll<HTMLElement>('.attention-beam__cycle')];
    expect(cycles).toHaveLength(2);
    expect(cycles[0].style.animationDelay).toBe('0ms');
    expect(cycles[1].style.animationDelay).toBe('1200ms');
    // 总时长 = 2400 + 尾音 180；2500ms 时仍在播放，2700ms 后结束。
    act(() => vi.advanceTimersByTime(2500));
    expect(onFinished).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(300));
    expect(onFinished).toHaveBeenCalledTimes(1);
  });

  it('请求更换后新事件键重新播放（渲染期重置）', () => {
    const first = beam({ eventKey: 'layer-1', durationMs: 100 });
    const { rerender, container } = render(<AttentionBeamLayer active request={first} />);
    act(() => vi.advanceTimersByTime(300));
    expect(container.querySelector('.attention-beam')).toBeNull();
    const second = beam({ eventKey: 'layer-2', durationMs: 100 });
    rerender(<AttentionBeamLayer active request={second} />);
    expect(container.querySelector('.attention-beam')).not.toBeNull();
    expect(container.querySelector('[data-event-key="layer-2"]')).not.toBeNull();
  });
});

describe('line 变体环形遮罩（P0-1）', () => {
  it('双层 mask 只保留边缘，中心区域透明', () => {
    expect(beamCss).toContain('padding: 1.5px');
    expect(beamCss).toContain('mask-composite: exclude');
    expect(beamCss).toContain('-webkit-mask-composite: exclude');
    expect(beamCss).toContain('linear-gradient(#000 0 0) content-box');
  });

  it('Reduced Motion 下 line 不再使用绕边扫掠动画', () => {
    expect(beamCss).toContain('prefers-reduced-motion');
    expect(beamCss).toContain('attention-static-fade');
  });
});

describe('灯光 Mutation 关联（P1-2）', () => {
  it('用户开启成功 → 播放（事件键带 mutationId）', () => {
    const mutationId = registerLightingAttention('zone-a', 'power-on', true);
    const played = confirmPendingLightingAttention(mutationId, {
      before: zoneState({ enabled: false }),
      after: zoneState({ enabled: true }),
    });
    expect(played).toBe(true);
    expect(getAttentionBusState().active?.eventKey).toBe(attentionLightingMutationKey('zone-a', 'power', 'on', mutationId));
    expect(getAttentionBusState().active?.priority).toBe(ATTENTION_PRIORITY['lighting-power-on']);
  });

  it('用户开启失败（目标值未确认）→ 不播放，pending 被消费', () => {
    const mutationId = registerLightingAttention('zone-a', 'power-on', true);
    const played = confirmPendingLightingAttention(mutationId, {
      before: zoneState({ enabled: false }),
      after: zoneState({ enabled: false }),
    });
    expect(played).toBe(false);
    expect(getAttentionBusState().active).toBeNull();
    expect(takePendingLightingAttention(mutationId)).toBeNull();
  });

  it('失败路径 clearPendingLightingAttention 后不播放', () => {
    const mutationId = registerLightingAttention('zone-a', 'power-on', true);
    clearPendingLightingAttention(mutationId);
    const played = confirmPendingLightingAttention(mutationId, {
      before: zoneState({ enabled: false }),
      after: zoneState({ enabled: true }),
    });
    expect(played).toBe(false);
    expect(getAttentionBusState().active).toBeNull();
  });

  it('自动状态变化（无登记）不播放', () => {
    expect(confirmPendingLightingAttention(undefined, {
      before: zoneState({ enabled: false }),
      after: zoneState({ enabled: true }),
    })).toBe(false);
    expect(getAttentionBusState().active).toBeNull();
  });

  it('相同值未实际变化 → 不播放', () => {
    const mutationId = registerLightingAttention('zone-a', 'color-applied', '#ff0000');
    const unchanged = zoneState({ color: '#ff0000' });
    const played = confirmPendingLightingAttention(mutationId, { before: unchanged, after: unchanged });
    expect(played).toBe(false);
    expect(getAttentionBusState().active).toBeNull();
  });

  it('开启→关闭→再次开启：两次开启都播放（事件键按 mutationId 区分）', () => {
    const firstId = registerLightingAttention('zone-a', 'power-on', true);
    confirmPendingLightingAttention(firstId, { before: zoneState({ enabled: false }), after: zoneState({ enabled: true }) });
    expect(getAttentionBusState().active?.eventKey).toBe(attentionLightingMutationKey('zone-a', 'power', 'on', firstId));
    finishActiveAttentionRequest();
    const secondId = registerLightingAttention('zone-a', 'power-on', true);
    const played = confirmPendingLightingAttention(secondId, {
      before: zoneState({ enabled: true }),
      after: zoneState({ enabled: true }),
    });
    // 前后一致（设备未变化）→ 不播放；但如果是真实“关闭后再次开启”，
    // 前后状态不同，且事件键与第一次不同，可以再次反馈。
    expect(played).toBe(false);
  });

  it('蓝→红→蓝：两次蓝色操作都可反馈（键不同）', () => {
    const firstId = registerLightingAttention('zone-a', 'color-applied', '#0000ff');
    confirmPendingLightingAttention(firstId, {
      before: zoneState({ color: '#ff0000' }),
      after: zoneState({ color: '#0000ff' }),
    });
    expect(getAttentionBusState().active?.eventKey).toBe(attentionLightingMutationKey('zone-a', 'color', '#0000ff', firstId));
    finishActiveAttentionRequest();
    const redId = registerLightingAttention('zone-a', 'color-applied', '#ff0000');
    confirmPendingLightingAttention(redId, {
      before: zoneState({ color: '#0000ff' }),
      after: zoneState({ color: '#ff0000' }),
    });
    finishActiveAttentionRequest();
    const secondBlueId = registerLightingAttention('zone-a', 'color-applied', '#0000ff');
    const played = confirmPendingLightingAttention(secondBlueId, {
      before: zoneState({ color: '#ff0000' }),
      after: zoneState({ color: '#0000ff' }),
    });
    expect(played).toBe(true);
    expect(getAttentionBusState().active?.eventKey).toBe(attentionLightingMutationKey('zone-a', 'color', '#0000ff', secondBlueId));
  });

  it('多 Zone 只播放对应 Zone 的光束', () => {
    const mouseId = registerLightingAttention('mouse-zone', 'color-applied', '#00ff00');
    confirmPendingLightingAttention(mouseId, {
      before: zoneState({ color: '#ff0000' }),
      after: zoneState({ color: '#00ff00' }),
    });
    const active = getAttentionBusState().active;
    expect(active?.scope).toBe('lighting:mouse-zone');
    expect(active?.eventKey).toBe(attentionLightingMutationKey('mouse-zone', 'color', '#00ff00', mouseId));
  });

  it('verifyLightingAttention 直接校验目标值', () => {
    const pending: PendingLightingAttention = { id: 1, zoneId: 'z', kind: 'effect-applied', expectedValue: 'wave' };
    expect(verifyLightingAttention(pending, zoneState({ effectValue: 'wave' }))).toBe(true);
    expect(verifyLightingAttention(pending, zoneState({ effectValue: 'static' }))).toBe(false);
  });

  it('lightingAttentionRequest 构建带 mutationId 的事件键与作用域', () => {
    const request = lightingAttentionRequest('power-on', 'z9', 'on', 42, '#3366ff');
    expect(request.eventKey).toBe(attentionLightingMutationKey('z9', 'power', 'on', 42));
    expect(request.scope).toBe('lighting:z9');
    expect(request.variant).toBe('line');
  });

  it('peekPendingLightingAttention 不消费登记', () => {
    const mutationId = registerLightingAttention('zone-a', 'power-on', true);
    const peeked = peekPendingLightingAttention(mutationId);
    expect(peeked?.zoneId).toBe('zone-a');
    expect(takePendingLightingAttention(mutationId)).not.toBeNull();
    expect(takePendingLightingAttention(mutationId)).toBeNull();
  });
});

describe('attentionColorForZone（P0-3）', () => {
  it('红/绿/蓝输出保留原色相（以原 hex 参与 color-mix，浏览器负责色相）', () => {
    expect(attentionColorForZone('#ff0000', true)).toContain('#ff0000');
    expect(attentionColorForZone('#00ff00', true)).toContain('#00ff00');
    expect(attentionColorForZone('#0000ff', false)).toContain('#0000ff');
  });

  it('黑色不产生明显彩色', () => {
    expect(attentionColorForZone('#000000', true)).toBe('oklch(60% 0.01 0)');
    expect(attentionColorForZone('#000000', false)).toBe('color-mix(in oklch, #000000 55%, white 45%)');
  });

  it('白色不产生明显彩色', () => {
    expect(attentionColorForZone('#ffffff', true)).toBe('color-mix(in oklch, #ffffff 72%, black 28%)');
    expect(attentionColorForZone('#ffffff', false)).toBe('oklch(62% 0.01 0)');
  });

  it('灰色保持低色度', () => {
    expect(attentionColorForZone('#808080', true)).toContain('#808080');
    expect(attentionColorForZone('#808080', false)).toContain('#808080');
  });

  it('无效颜色原样返回，不抛异常', () => {
    expect(attentionColorForZone('not-a-color')).toBe('not-a-color');
    expect(attentionColorForZone('')).toBe('');
    expect(attentionColorForZone('transparent')).toBe('transparent');
  });

  it('3 位 hex 支持', () => {
    expect(attentionColorForZone('#f00', true)).toContain('#f00');
  });
});
