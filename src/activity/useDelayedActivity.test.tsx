// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useDelayedActivity } from './useDelayedActivity';

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(performance, 'now').mockReturnValue(0);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe('useDelayedActivity', () => {
  it('does not show a fast operation', () => {
    const { result, rerender } = renderHook(
      ({ active }) => useDelayedActivity(active, 300, 420),
      { initialProps: { active: true } },
    );

    act(() => vi.advanceTimersByTime(250));
    expect(result.current).toBe(false);

    rerender({ active: false });
    act(() => vi.runAllTimers());
    expect(result.current).toBe(false);
  });

  it('shows immediately when a zero-delay activity starts after mount', () => {
    const { result, rerender } = renderHook(
      ({ active }) => useDelayedActivity(active, 0, 420),
      { initialProps: { active: false } },
    );

    expect(result.current).toBe(false);
    rerender({ active: true });
    expect(result.current).toBe(true);
  });

  it('keeps a shown operation visible for its minimum duration', () => {
    const now = vi.mocked(performance.now);
    const { result, rerender } = renderHook(
      ({ active }) => useDelayedActivity(active, 300, 420),
      { initialProps: { active: true } },
    );

    act(() => {
      now.mockReturnValue(300);
      vi.advanceTimersByTime(300);
    });
    expect(result.current).toBe(true);

    act(() => now.mockReturnValue(500));
    rerender({ active: false });
    act(() => {
      vi.advanceTimersByTime(219);
    });
    expect(result.current).toBe(true);

    act(() => vi.advanceTimersByTime(2));
    expect(result.current).toBe(false);
  });
});
