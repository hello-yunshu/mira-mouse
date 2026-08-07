// SPDX-License-Identifier: AGPL-3.0-or-later
// AttentionBusController 全局生命周期测试（§11.1）：
// - A 无匹配 Scope 自动结束（总线不锁死）；
// - B pending 自动推进；
// - C 旧 Timer 不结束新请求；
// - D Surface 卸载不提前结束；
// - E Reduced Motion 统一时长（不乘以 cycles）；
// - F Strict Mode 不双 finish / 不跳 pending。

import { StrictMode } from 'react';
import { act, render, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  announceAttentionRequest,
  finishActiveAttentionRequest,
  getAttentionBusState,
  resetAttentionBusForTests,
} from './attentionCore';
import { AttentionBusController } from './AttentionBusController';
import { useAttentionFeedback } from './useAttentionFeedback';
import {
  ATTENTION_BEAM_TAIL_MS,
  ATTENTION_REDUCED_MOTION_TOTAL_MS,
  attentionRequestTotalMs,
  prefersReducedAttentionMotion,
} from './attentionTiming';
import type { AttentionBeamRequest } from './attentionTypes';

function beam(overrides: Partial<AttentionBeamRequest> = {}): AttentionBeamRequest {
  return {
    eventKey: `controller:${Math.random()}`,
    scope: 'surface:not-mounted',
    variant: 'line',
    color: '#ffb3b3',
    durationMs: 300,
    strength: 0.2,
    cycles: 1,
    priority: 50,
    ...overrides,
  };
}

function setReducedMotion(enabled: boolean): void {
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: enabled && query.includes('prefers-reduced-motion'),
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }));
}

beforeEach(() => {
  resetAttentionBusForTests();
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('attentionTiming 统一时长', () => {
  it('normalTotal = delay + duration + tail，不乘以 cycles', () => {
    expect(attentionRequestTotalMs({ durationMs: 2400 }, false)).toBe(2400 + ATTENTION_BEAM_TAIL_MS);
    expect(attentionRequestTotalMs({ delayMs: 100, durationMs: 800 }, false)).toBe(100 + 800 + ATTENTION_BEAM_TAIL_MS);
  });

  it('Reduced Motion 截断到固定总时长', () => {
    expect(attentionRequestTotalMs({ durationMs: 2400 }, true)).toBe(ATTENTION_REDUCED_MOTION_TOTAL_MS);
    expect(attentionRequestTotalMs({ durationMs: 200 }, true)).toBe(200 + ATTENTION_BEAM_TAIL_MS);
  });

  it('负时长不允许：归零后仍为正常数值', () => {
    expect(attentionRequestTotalMs({ delayMs: -5, durationMs: -1 }, false)).toBe(ATTENTION_BEAM_TAIL_MS);
  });

  it('prefersReducedAttentionMotion 读取系统偏好', () => {
    const originalMatchMedia = window.matchMedia;
    setReducedMotion(true);
    expect(prefersReducedAttentionMotion()).toBe(true);
    setReducedMotion(false);
    expect(prefersReducedAttentionMotion()).toBe(false);
    window.matchMedia = originalMatchMedia;
  });
});

describe('AttentionBusController', () => {
  it('A：无匹配 Scope 的请求到时自动结束，总线不锁死', () => {
    render(<AttentionBusController />);
    act(() => {
      announceAttentionRequest(beam({ eventKey: 'unknown-scope', scope: 'surface:not-mounted', durationMs: 300 }));
    });
    expect(getAttentionBusState().active?.eventKey).toBe('unknown-scope');
    act(() => vi.advanceTimersByTime(600));
    expect(getAttentionBusState().active).toBeNull();
    expect(getAttentionBusState().pending).toEqual([]);
  });

  it('B：pending 自动推进，两个无 Scope 请求都依次结束', () => {
    render(<AttentionBusController />);
    act(() => {
      announceAttentionRequest(beam({ eventKey: 'req-a', durationMs: 300 }));
      announceAttentionRequest(beam({ eventKey: 'req-b', durationMs: 300 }));
    });
    expect(getAttentionBusState().active?.eventKey).toBe('req-a');
    expect(getAttentionBusState().pending.map((item) => item.eventKey)).toEqual(['req-b']);
    // A 到期 → B 成为 active。
    act(() => vi.advanceTimersByTime(600));
    expect(getAttentionBusState().active?.eventKey).toBe('req-b');
    // B 到期 → 总线为空。
    act(() => vi.advanceTimersByTime(600));
    expect(getAttentionBusState().active).toBeNull();
    expect(getAttentionBusState().pending).toEqual([]);
  });

  it('C：旧 Timer 不结束新请求', () => {
    render(<AttentionBusController />);
    // 旧请求总时长 680ms，新请求总时长 1080ms，用时间差证明计时器绑定到各自的请求。
    act(() => {
      announceAttentionRequest(beam({ eventKey: 'old-req', durationMs: 500 }));
    });
    expect(getAttentionBusState().active?.eventKey).toBe('old-req');
    // 手动结束 A，B 立即成为 active。
    act(() => {
      finishActiveAttentionRequest();
      announceAttentionRequest(beam({ eventKey: 'new-req', durationMs: 900 }));
    });
    expect(getAttentionBusState().active?.eventKey).toBe('new-req');
    // 推进到 A 的原结束时间（700 > 680）：B 必须仍然 active。
    act(() => vi.advanceTimersByTime(700));
    expect(getAttentionBusState().active?.eventKey).toBe('new-req');
    // B 自己的总时长（1080ms）结束后才结束。
    act(() => vi.advanceTimersByTime(500));
    expect(getAttentionBusState().active).toBeNull();
  });

  it('D：Surface 卸载不提前结束，到全局时间才结束', () => {
    render(<AttentionBusController />);
    const { result, unmount } = renderHook(() => useAttentionFeedback('surface:d'));
    act(() => {
      announceAttentionRequest(beam({ eventKey: 'surface-req', scope: 'surface:d', durationMs: 300 }));
    });
    expect(result.current.beam?.eventKey).toBe('surface-req');
    // Surface 卸载：active 必须仍然存在。
    unmount();
    expect(getAttentionBusState().active?.eventKey).toBe('surface-req');
    // 全局时间到期才结束。
    act(() => vi.advanceTimersByTime(600));
    expect(getAttentionBusState().active).toBeNull();
  });

  it('E：Reduced Motion 下约 450ms 结束，不乘以 cycles', () => {
    const originalMatchMedia = window.matchMedia;
    setReducedMotion(true);
    render(<AttentionBusController />);
    act(() => {
      announceAttentionRequest(beam({ eventKey: 'rm-req', durationMs: 2400, cycles: 2 }));
    });
    expect(getAttentionBusState().active?.eventKey).toBe('rm-req');
    act(() => vi.advanceTimersByTime(ATTENTION_REDUCED_MOTION_TOTAL_MS + 50));
    expect(getAttentionBusState().active).toBeNull();
    window.matchMedia = originalMatchMedia;
  });

  it('F：Strict Mode 不双 finish、不跳 pending，总线最终归空', () => {
    render(
      <StrictMode>
        <AttentionBusController />
      </StrictMode>,
    );
    act(() => {
      announceAttentionRequest(beam({ eventKey: 'strict-a', durationMs: 300 }));
      announceAttentionRequest(beam({ eventKey: 'strict-b', durationMs: 300 }));
    });
    expect(getAttentionBusState().active?.eventKey).toBe('strict-a');
    act(() => vi.advanceTimersByTime(600));
    // 只推进一次：B 成为 active（没有被双 finish 跳过）。
    expect(getAttentionBusState().active?.eventKey).toBe('strict-b');
    act(() => vi.advanceTimersByTime(600));
    expect(getAttentionBusState().active).toBeNull();
    expect(getAttentionBusState().pending).toEqual([]);
  });
});
