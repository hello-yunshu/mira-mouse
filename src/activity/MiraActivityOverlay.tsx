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
  beginActivityTask,
  registerVisibleActivity,
  unregisterVisibleActivity,
  useActiveBeamForScope,
  useActivityExitHint,
  type ActivityRegistrationToken,
  type ActivityScope,
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
 * 全局 64px Orb 的可见状态与协调 scope。
 *
 * 协调 scope 的语义：业务状态（detected）已经变为 null 时，只要仍有一个
 * 正在显示的 Orb（可能处于最短可见尾段），scope 就继续跟随 displayed 的
 * 对应 scope，直到该 Orb 真实退出；否则同 scope 完成事件（Beam）发出的
 * “立即退出”提示会失去订阅目标，Orb 只能自然等完 420ms 尾段，完成反馈被
 * 无谓延迟。业务状态为 null 且没有任何 displayed Orb 时，scope 才真正归零。
 *
 * 可见状态语义：延迟出现、最短可见、同 scope 完成事件出现时立即退出
 * （跳过最短可见尾段）。被强制退出后，在业务状态真正结束（变为 null 或
 * 换成另一种活动）之前不再回弹。所有 setState 都发生在定时器回调里，
 * 避免 set-state-in-effect。
 */
function useGlobalActivityCoordination(
  detected: MiraGlobalActivity,
): {
  displayed: MiraGlobalActivity;
  coordinationScope: ActivityScope | null;
} {
  // 初始不跟随检测值：显式业务状态也必须经过 300ms 延迟出现在先，
  // 否则 App 一挂载就闪现 Orb。
  const [displayed, setDisplayed] = useState<MiraGlobalActivity>(null);
  const detectedScope = detected ? attentionScopeForActivity(detected) : null;
  // displayed 是上一轮渲染的 state，本轮即可参与 scope 计算，无循环依赖。
  const displayedScope = displayed
    ? attentionScopeForActivity(displayed)
    : null;
  // 有 displayed Activity 时完全尊重其自身 scope（即使是无注意力的 null）；
  // 只有没有 displayed 时才回退到业务检测到的 scope。
  const coordinationScope = displayed !== null
    ? displayedScope
    : detectedScope;
  const exitHint = useActivityExitHint(coordinationScope);
  const visibleSinceRef = useRef(0);
  const lastExitHintRef = useRef(0);
  // 同一 scope 重新建立（null → 再次进入）时，退出提示相对基线要一起重置，
  // 否则第二轮 ready 的增量 0→1 会被上一轮消费值 1 挡住，Orb 只能自然等完
  // 最短可见尾段，完成反馈被无谓延迟。
  const lastExitHintScopeRef = useRef<ActivityScope | null>(coordinationScope);
  // 被同 scope 完成事件强制退出后，在业务状态真正结束之前不再回弹。
  const suppressedValueRef = useRef<MiraGlobalActivity>(null);

  useEffect(() => {
    let timer = 0;

    if (lastExitHintScopeRef.current !== coordinationScope) {
      // scope 切换/重新建立时同步重置退出提示基线：`useActivityExitHint`
      // 已在切换渲染返回 0，这里跟随重定基线，防止历史消费值挡掉新提示。
      lastExitHintScopeRef.current = coordinationScope;
      lastExitHintRef.current = exitHint;
    }

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
  }, [detected, displayed, coordinationScope, exitHint]);

  return { displayed, coordinationScope };
}

export interface MiraActivityOverlayProps {
  /**
   * App 显式计算得到的全局业务状态（第一优先级）。未提供该属性时才允许
   * DOM 兜底检测；显式 null 表示“当前没有活动”，不会被 DOM 标志覆盖。
   */
  activity?: MiraGlobalActivity | null;
}

export function MiraActivityOverlay({
  activity: deviceActivity,
}: MiraActivityOverlayProps) {
  const { i18n } = useTranslation();
  const hasExplicitActivity = deviceActivity !== undefined;
  const detectedDom = useDetectedGlobalActivity(!hasExplicitActivity);
  const detected = hasExplicitActivity ? deviceActivity : detectedDom;
  const { displayed, coordinationScope } = useGlobalActivityCoordination(detected);
  // 渲染层兜底仲裁：同一 scope 已有 Beam 播放时不显示全局 Orb。
  const beamActive = useActiveBeamForScope(coordinationScope);
  const showOrb = displayed !== null && !beamActive;
  // 与 Inline 组件一致：以组件级 token 注册当前显示的 Orb。
  const [token] = useState<ActivityRegistrationToken>(
    () => Symbol('mira-activity-overlay'),
  );

  // 业务状态（detected）在第 0ms 开始（早于 Orb 300ms 可见），任务开始时
  // 递增代数，供 announceAfterOrbExit 在等待期间判断是否已开始新一代任务。
  useEffect(() => {
    const scope = detected ? attentionScopeForActivity(detected) : null;
    if (scope) beginActivityTask(scope);
  }, [detected]);

  useEffect(() => {
    if (!coordinationScope) return;
    if (showOrb) registerVisibleActivity(coordinationScope, token);
    // 清理覆盖 scope 切换与卸载：上一轮已注册的令牌在这里同步注销。
    return () => {
      if (showOrb) unregisterVisibleActivity(coordinationScope, token);
    };
  }, [coordinationScope, showOrb, token]);

  if (!showOrb) return null;

  const spec = miraActivitySpec(displayed);
  const label = miraActivityLabel(
    displayed,
    i18n.resolvedLanguage ?? i18n.language,
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
