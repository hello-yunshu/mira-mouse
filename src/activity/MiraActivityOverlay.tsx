// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from 'react';
import { ThinkingOrb } from 'thinking-orbs';
import { useTranslation } from 'react-i18next';
import { OverlayPortal } from '../overlay';
import {
  MIRA_ACTIVITY_MIN_VISIBLE_MS,
  MIRA_ACTIVITY_SHOW_DELAY_MS,
  miraActivityLabel,
  miraActivitySpec,
  resolveGlobalMiraActivity,
  type MiraGlobalActivity,
} from './activityCatalog';
import {
  attentionScopeForActivity,
  beginActivity,
  endActivity,
  useActiveBeamForScope,
  useActivityExitHint,
} from './activityCoordinator';

/**
 * DOM 兼容兜底：仅当未提供显式业务状态时使用 MutationObserver 扫描。
 * 显式状态（App 传入的 activity）为第一优先级。
 */
function useDetectedGlobalActivity(): MiraGlobalActivity {
  const [activity, setActivity] = useState<MiraGlobalActivity>(() => (
    typeof document === 'undefined'
      ? null
      : resolveGlobalMiraActivity(document)
  ));

  useEffect(() => {
    if (typeof document === 'undefined' || !document.body) return;
    if (typeof MutationObserver === 'undefined') return;

    let frame = 0;
    const sync = () => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        const next = resolveGlobalMiraActivity(document);
        setActivity((current) => current === next ? current : next);
      });
    };

    const observer = new MutationObserver(sync);
    observer.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['class'],
    });
    sync();

    return () => {
      observer.disconnect();
      window.cancelAnimationFrame(frame);
    };
  }, []);

  return activity;
}

/**
 * 全局 64px Orb 的可见状态：延迟出现、最短可见、同 scope 完成事件出现时
 * 立即退出（跳过最短可见尾段）。
 * 所有 setState 都发生在定时器回调里，避免 set-state-in-effect。
 */
function useDisplayedGlobalActivity(
  detected: MiraGlobalActivity,
  exitHint: number,
): MiraGlobalActivity {
  // 初始不跟随检测值：显式业务状态也必须经过 300ms 延迟出现在先，
  // 否则 App 一挂载就闪现 Orb（P0-3 第 1 条）。
  const [displayed, setDisplayed] = useState<MiraGlobalActivity>(null);
  const visibleSinceRef = useRef(0);
  const lastExitHintRef = useRef(0);
  // 被同 scope 完成事件强制退出后，在业务状态真正结束（变为 null 或换成
  // 另一种活动）之前不再回弹。
  const suppressedValueRef = useRef<MiraGlobalActivity>(null);

  useEffect(() => {
    let timer = 0;

    if (exitHint > lastExitHintRef.current) {
      lastExitHintRef.current = exitHint;
      suppressedValueRef.current = detected;
      timer = window.setTimeout(() => setDisplayed(null), 0);
    } else if (detected) {
      if (detected !== suppressedValueRef.current && !displayed) {
        timer = window.setTimeout(() => {
          visibleSinceRef.current = performance.now();
          setDisplayed(detected);
        }, Math.max(0, MIRA_ACTIVITY_SHOW_DELAY_MS));
      } else if (displayed && displayed !== detected) {
        timer = window.setTimeout(() => setDisplayed(detected), 0);
      }
    } else {
      // 业务状态已结束：解除强制退出抑制，允许下一个任务正常出现。
      suppressedValueRef.current = null;
      if (displayed) {
        const elapsed = performance.now() - visibleSinceRef.current;
        timer = window.setTimeout(
          () => setDisplayed(null),
          Math.max(0, MIRA_ACTIVITY_MIN_VISIBLE_MS - elapsed),
        );
      }
    }

    return () => window.clearTimeout(timer);
  }, [detected, displayed, exitHint]);

  return displayed;
}

export type MiraDeviceActivity = Exclude<MiraGlobalActivity, 'battery-analysis'>;

export interface MiraActivityOverlayProps {
  /**
   * App 显式计算的全局业务状态（第一优先级）。未提供时回退到 DOM 检测
   * （电量弹窗仍始终走 DOM 判定，因为它覆盖在 Dashboard 之上）。
   */
  activity?: MiraGlobalActivity | null;
  /** 电量功能真实启用状态，用于区分电量分析文案。 */
  batteryAnalysisEnabled?: boolean;
}

export function MiraActivityOverlay({
  activity: deviceActivity,
  batteryAnalysisEnabled = false,
}: MiraActivityOverlayProps) {
  const { i18n } = useTranslation();
  const detectedDom = useDetectedGlobalActivity();
  const detected = deviceActivity ?? detectedDom;
  const detectedScope = detected ? attentionScopeForActivity(detected) : null;
  const exitHint = useActivityExitHint(detectedScope);
  const displayed = useDisplayedGlobalActivity(detected, exitHint);
  const displayedScope = displayed ? attentionScopeForActivity(displayed) : null;
  // 渲染层兜底仲裁：同一 scope 已有 Beam 播放时不显示全局 Orb。
  const beamActive = useActiveBeamForScope(displayedScope);
  const showOrb = displayed !== null && !beamActive;

  useEffect(() => {
    if (!displayedScope) return;
    if (showOrb) beginActivity(displayedScope);
    else endActivity(displayedScope);
  }, [displayedScope, showOrb]);

  useEffect(() => {
    if (!displayedScope) return;
    return () => endActivity(displayedScope);
  }, [displayedScope]);

  if (!showOrb) return null;

  const spec = miraActivitySpec(displayed);
  const label = miraActivityLabel(
    displayed,
    i18n.resolvedLanguage ?? i18n.language,
    { aiAnalysisEnabled: batteryAnalysisEnabled },
  );

  return (
    <OverlayPortal>
      <div
        className="mira-activity-overlay"
        data-mira-activity={displayed}
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        <div className="mira-activity-card">
          <ThinkingOrb
            state={spec.state}
            size={spec.size}
            speed={spec.speed}
            theme="auto"
            aria-hidden="true"
          />
          <span>{label}</span>
        </div>
      </div>
    </OverlayPortal>
  );
}