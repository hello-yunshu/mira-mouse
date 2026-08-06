// SPDX-License-Identifier: AGPL-3.0-or-later
// P0-3：Orb 与 Attention Beam 生命周期边界的关键集成测试。
// mock 第三方视觉组件（thinking-orbs），不测试 Canvas 绘制。
import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MiraActivityOverlay } from './MiraActivityOverlay';
import { MiraInlineActivity } from './MiraInlineActivity';
import {
  announceAfterOrbExit,
  isActivityVisible,
  resetActivityCoordinatorForTests,
} from './activityCoordinator';
import { announceAttentionRequest, resetAttentionBusForTests } from '../attention/attentionCore';
import type { AttentionBeamRequest } from '../attention/attentionTypes';

vi.mock('thinking-orbs', () => ({
  ThinkingOrb: ({ state }: { state: string }) => (
    <span data-testid="thinking-orb" data-state={state} />
  ),
}));

function request(
  eventKey: string,
  scope: string,
  priority = 10,
  variant: AttentionBeamRequest['variant'] = 'line',
): AttentionBeamRequest {
  return {
    eventKey,
    scope,
    variant,
    color: '#ffb3b3',
    durationMs: 1200,
    strength: 0.16,
    cycles: 1,
    priority,
  };
}

describe('MiraActivityOverlay 生命周期仲裁（P0-3）', () => {
  beforeEach(() => {
    vi.useFakeTimers({
      toFake: [
        'setTimeout', 'clearTimeout', 'setInterval', 'clearInterval',
        'Date', 'performance', 'requestAnimationFrame', 'cancelAnimationFrame',
      ],
    });
    resetActivityCoordinatorForTests();
    resetAttentionBusForTests();
  });

  afterEach(() => {
    vi.useRealTimers();
    resetActivityCoordinatorForTests();
    resetAttentionBusForTests();
  });

  it('1. 设备初始化超过 300ms 才显示 Orb', () => {
    render(<MiraActivityOverlay activity="device-initializing" />);
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(200); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(150); });
    expect(screen.getByTestId('thinking-orb')).toHaveAttribute('data-state', 'connecting');
    expect(screen.getByRole('status')).toHaveTextContent('正在识别并读取鼠标…');
  });

  it('2. 设备 ready 后 Orb 立即退出（不等 420ms 尾段），随后才播放 ready Beam', async () => {
    const announce = vi.fn(() => true);
    render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    act(() => {
      void announceAfterOrbExit('device:app', announce, request('ready-1', 'device:app'));
    });
    expect(announce).not.toHaveBeenCalled();
    // 立即退出提示（0ms）+ rAF（~16ms）后，Orb 先退出，Beam 才提交。
    await act(async () => {
      vi.advanceTimersByTime(100);
      await Promise.resolve();
    });
    expect(announce).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    // 业务状态未结束也不得回弹（suppression）。
    act(() => { vi.advanceTimersByTime(1000); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('3. ready Beam 已活跃时，Orb 不出现（渲染层兜底仲裁）', () => {
    act(() => {
      announceAttentionRequest(request('ready-1', 'device:app'));
    });
    render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(1000); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('4. 等待鼠标就位显示 connecting Orb 与等待文案', () => {
    render(<MiraActivityOverlay activity="awaiting-mouse" />);
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toHaveAttribute('data-state', 'connecting');
    expect(screen.getByRole('status')).toHaveTextContent('正在等待鼠标就位…');
  });

  it('5. 更新检查完成、available Beam 播放时，检查 Orb 不存在', () => {
    render(
      <MiraInlineActivity active activity="checking-app-update" />,
    );
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    act(() => {
      announceAttentionRequest(request('app-update-1', 'about-update'));
    });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('6. installed / restart Beam 同样不与 Orb 重叠（pulse-inner 变体）', () => {
    act(() => {
      announceAttentionRequest(request('restart-1', 'about-update', 20, 'pulse-inner'));
    });
    render(<MiraInlineActivity active activity="checking-app-update" />);
    act(() => { vi.advanceTimersByTime(1000); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('7. 不同 scope 的任务互不误伤', () => {
    act(() => {
      announceAttentionRequest(request('plugin-update-1', 'settings-plugin'));
    });
    render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
  });

  it('8. 卸载后 timer/rAF/observer 清理，协调层状态清空', () => {
    const { unmount } = render(<MiraActivityOverlay activity="device-initializing" />);
    act(() => { vi.advanceTimersByTime(350); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();
    expect(isActivityVisible('device:app')).toBe(true);

    unmount();
    expect(isActivityVisible('device:app')).toBe(false);
    // 继续推进时间：不应再有状态更新或报错。
    expect(() => { act(() => { vi.advanceTimersByTime(5000); }); }).not.toThrow();
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('9. 快任务低于 300ms：Orb 不出现', () => {
    const { rerender } = render(<MiraInlineActivity active activity="scanning-devices" />);
    act(() => { vi.advanceTimersByTime(200); });
    rerender(<MiraInlineActivity active={false} activity="scanning-devices" />);
    act(() => { vi.advanceTimersByTime(1000); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });

  it('10. 无 Beam 的普通任务仍满足 420ms 最短可见', () => {
    const { rerender } = render(<MiraInlineActivity active activity="scanning-devices" />);
    act(() => { vi.advanceTimersByTime(300); });
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    rerender(<MiraInlineActivity active={false} activity="scanning-devices" />);
    act(() => { vi.advanceTimersByTime(300); });
    // 已过 600ms，距出现仅 300ms，仍应可见（最短 420ms）。
    expect(screen.getByTestId('thinking-orb')).toBeInTheDocument();

    act(() => { vi.advanceTimersByTime(200); });
    expect(screen.queryByTestId('thinking-orb')).not.toBeInTheDocument();
  });
});
