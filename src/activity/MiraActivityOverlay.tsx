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
  registerVisibleActivity,
  unregisterVisibleActivity,
  useActiveBeamForScope,
  useActivityExitHint,
  type ActivityRegistrationToken,
} from './activityCoordinator';

/**
 * DOM 兼容兜底：仅当未提供显式业务状态时才启用 MutationObserver 扫描。
 * 显式状态（App 传入的 activity）为第一优先级，显式模式下不监听整个
 * document.body。
 */
function useDetectedGlobalActivity(enabled: boolean): MiraGlobalActivity {
  const [activity, setActivity] = useState<MiraGlobalActivity>(() => (
    !enabled || typeof document === 'undefined'
      ? null
      : resolveGlobalMiraActivity(document)
  ));

  useEffect(() => {
    if (!enabled) return;
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
  }, [enabled]);

  return activity;
}

/**
 * 全局 64px Orb 的可见状态：延迟出现、最短可见、同 scope 完成事件出现时
 * 立即退出（跳过最短可见尾段）。被强制退出后，在业务状态真正结束（变为
 * null 或换成另一种活动）之前不再回弹。
 * 所有 setState 都发生在定时器回调里，避免 set-state-in-effect。
 */
function useDisplayedGlobalActivity(
  detected: MiraGlobalActivity,
  exitHint: number,
): MiraGlobalActivity {
  // 初始不跟随检测值：显式业务状态也必须经过 300ms 延迟出现在先，
  // 否则 App 一挂载就闪现 Orb。
  const [displayed, setDisplayed] = useState<MiraGlobalActivity>(null);
  const visibleSinceRef = useRef(0);
  const lastExitHintRef = useRef(0);
  // 被同 scope 完成事件强制退出后，在业务状态真正结束之前不再回弹。
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

export interface MiraActivityOverlayProps {
  /**
   * App 显式计算得到的全局业务状态（第一优先级）。未提供该属性时才允许
   * DOM 兜底检测；显式 null 表示“当前没有活动”，不会被 DOM 标志覆盖。
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
  const hasExplicitActivity = deviceActivity !== undefined;
  const detectedDom = useDetectedGlobalActivity(!hasExplicitActivity);
  const detected = hasExplicitActivity ? deviceActivity : detectedDom;
  const detectedScope = detected ? attentionScopeForActivity(detected) : null;
  const exitHint = useActivityExitHint(detectedScope);
  const displayed = useDisplayedGlobalActivity(detected, exitHint);
  const displayedScope = displayed ? attentionScopeForActivity(displayed) : null;
  // 渲染层兜底仲裁：同一 scope 已有 Beam 播放时不显示全局 Orb。
  const beamActive = useActiveBeamForScope(displayedScope);
  const showOrb = displayed !== null && !beamActive;
  // 与 Inline 组件一致：以组件级 token 注册当前显示的 Orb。
  const [token] = useState<ActivityRegistrationToken>(
    () => Symbol('mira-activity-overlay'),
  );

  useEffect(() => {
    if (!displayedScope) return;
    if (showOrb) registerVisibleActivity(displayedScope, token);
    // 清理覆盖 scope 切换与卸载：上一轮已注册的令牌在这里同步注销。
    return () => {
      if (showOrb) unregisterVisibleActivity(displayedScope, token);
    };
  }, [displayedScope, showOrb, token]);

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