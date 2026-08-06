// SPDX-License-Identifier: AGPL-3.0-or-later
// Attention Beam 模块测试：总线仲裁、会话去重、设备状态机、
// useAttentionFeedback 作用域过滤、AttentionBeamLayer 渲染与卸载、
// LightingAttention 信号迁移观察器。

import { act, render } from '@testing-library/react';
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
  attentionLightingKey,
  reduceDeviceAttention,
  type AttentionBeamRequest,
  type DeviceAttentionContext,
} from './attentionTypes';
import { AttentionBeamLayer } from './AttentionBeamLayer';
import { LightingAttention } from './LightingAttention';
import { useAttentionFeedback } from './useAttentionFeedback';

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

beforeEach(() => {
  resetAttentionBusForTests();
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
      request: beam({ eventKey: 'first', priority: 20 }),
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

describe('reduceDeviceAttention 设备就绪状态机', () => {
  it('首次观察不触发任何事件（启动即在线不算连接）', () => {
    const outcome = reduceDeviceAttention(initialContext(), true, 0);
    expect(outcome.action).toBe('none');
    expect(outcome.wasReady).toBe(true);
  });

  it('启动离线后首次就绪视为 device-ready（宽限期内也有效）', () => {
    const first = reduceDeviceAttention(initialContext(), false, 0);
    const second = reduceDeviceAttention(
      { ...initialContext(), previous: false, wasReady: false },
      true,
      1000,
    );
    expect(first.action).toBe('none');
    expect(second.action).toBe('ready');
  });

  it('就绪后断开再恢复视为 device-reconnected 并递增周期', () => {
    let context: DeviceAttentionContext = { previous: true, wasReady: true, cycle: 0 };
    const dropped = reduceDeviceAttention(context, false, 5000);
    expect(dropped.action).toBe('none');
    context = { previous: false, wasReady: true, cycle: dropped.cycle };
    const restored = reduceDeviceAttention(context, true, 6000);
    expect(restored.action).toBe('reconnected');
    expect(restored.cycle).toBe(1);
  });

  it('状态未变化时无操作', () => {
    const context: DeviceAttentionContext = { previous: true, wasReady: true, cycle: 2 };
    expect(reduceDeviceAttention(context, true, 999).action).toBe('none');
  });
});

describe('useAttentionFeedback', () => {
  it('只渲染本作用域的光束', async () => {
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
    expect(result.current.announce(beam({ eventKey: attentionLightingKey('z1', 'power', 'on') }))).toBe(true);
  });

  it('behavior=high 拒绝灯光类事件', () => {
    const { result } = renderHook(() => useAttentionFeedback('x', { behavior: 'high' }));
    expect(result.current.announce(beam({ eventKey: attentionLightingKey('z1', 'power', 'on') }))).toBe(false);
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
      <AttentionBeamLayer active request={beam({ variant: 'pulse-inner', color: '#123456', cycles: 2 })} />,
    );
    const element = container.querySelector('.attention-beam');
    expect(element?.className).toContain('attention-beam--pulse-inner');
    expect((element as HTMLElement).style.getPropertyValue('--beam-color')).toBe('#123456');
    expect(element?.querySelectorAll('.attention-beam__cycle')).toHaveLength(2);
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

describe('LightingAttention 信号迁移观察器', () => {
  function zoneProps(overrides: { zoneId?: string; enabled?: boolean; color?: string; effectValue?: unknown } = {}) {
    return {
      zoneId: 'zone-a',
      enabled: false,
      color: '',
      effectValue: undefined,
      ...overrides,
    };
  }

  it('首次挂载已有状态不触发事件', () => {
    const { rerender } = render(<LightingAttention {...zoneProps({ enabled: true, color: '#ff0000' })} />);
    rerender(<LightingAttention {...zoneProps({ enabled: true, color: '#ff0000' })} />);
    expect(getAttentionBusState().active).toBeNull();
  });

  it('关闭→开启触发 lighting:power-on', () => {
    const { rerender } = render(<LightingAttention {...zoneProps({ enabled: false })} />);
    rerender(<LightingAttention {...zoneProps({ enabled: true, color: '#00ff00' })} />);
    const active = getAttentionBusState().active;
    expect(active?.eventKey).toBe(attentionLightingKey('zone-a', 'power', 'on'));
    expect(active?.priority).toBe(ATTENTION_PRIORITY['lighting-power-on']);
  });

  it('颜色确认写入新值触发 lighting:color-applied，同色不重复触发', () => {
    const { rerender } = render(<LightingAttention {...zoneProps({ enabled: true, color: '#00ff00' })} />);
    rerender(<LightingAttention {...zoneProps({ enabled: true, color: '#3366ff' })} />);
    expect(getAttentionBusState().active?.eventKey).toBe(attentionLightingKey('zone-a', 'color', '#3366ff'));
    finishActiveAttentionRequest();
    rerender(<LightingAttention {...zoneProps({ enabled: true, color: '#3366ff' })} />);
    expect(getAttentionBusState().active).toBeNull();
  });

  it('effect 字段变化触发 lighting:effect-applied', () => {
    const { rerender } = render(<LightingAttention {...zoneProps({ enabled: true, effectValue: 1 })} />);
    rerender(<LightingAttention {...zoneProps({ enabled: true, effectValue: 2 })} />);
    const active = getAttentionBusState().active;
    expect(active?.eventKey).toBe(attentionLightingKey('zone-a', 'effect', '2'));
    expect(active?.priority).toBe(ATTENTION_PRIORITY['lighting-effect-applied']);
  });

  it('切换 Zone 时已有状态不算事件', () => {
    const { rerender } = render(<LightingAttention {...zoneProps({ zoneId: 'zone-a', enabled: true, color: '#ff0000' })} />);
    rerender(<LightingAttention {...zoneProps({ zoneId: 'zone-b', enabled: true, color: '#ff0000' })} />);
    expect(getAttentionBusState().active).toBeNull();
  });
});

describe('attentionColorForZone', () => {
  it('无效输入原样返回', () => {
    expect(attentionColorForZone('not-a-color')).toBe('not-a-color');
  });

  it('合法 hex 输出 oklch 串', () => {
    expect(attentionColorForZone('#3366ff', false)).toMatch(/^oklch\(/);
  });
});

// renderHook 由 @testing-library/react 提供
import { renderHook } from '@testing-library/react';
