// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect } from 'react';
import { ThinkingOrb } from 'thinking-orbs';
import { useTranslation } from 'react-i18next';
import {
  miraActivityLabel,
  miraActivitySpec,
  type MiraActivityKind,
} from './activityCatalog';
import { useDelayedActivity } from './useDelayedActivity';
import {
  attentionScopeForActivity,
  beginActivity,
  endActivity,
  useActiveBeamForScope,
  useActivityExitHint,
  type ActivityScope,
} from './activityCoordinator';

export interface MiraInlineActivityProps {
  active: boolean;
  activity: MiraActivityKind;
  className?: string;
  /**
   * 按钮文案已经表达“正在……”时保持 false，避免屏幕阅读器重复播报。
   * 图标单独承担状态表达时可设为 true。
   */
  announce?: boolean;
  delayMs?: number;
  minVisibleMs?: number;
  /**
   * 空闲与延迟出现期间（0–300ms）显示的图标。提供后空闲期不再保留
   * 不可见的空槽，图标平滑替换成 Orb，结束后恢复图标。
   */
  fallback?: React.ReactNode;
  /** 与 Attention Beam 仲裁的作用域；没有对应完成事件时可省略。 */
  scope?: ActivityScope;
  /**
   * Idle 时仍保留 20px 空槽，维持按钮宽度不变。
   * 默认关闭：无原图标的按钮不再长期保留透明空白位。
   */
  reserveSpace?: boolean;
}

export function MiraInlineActivity({
  active,
  activity,
  className,
  announce = false,
  delayMs,
  minVisibleMs,
  fallback,
  scope,
  reserveSpace = false,
}: MiraInlineActivityProps) {
  const { i18n } = useTranslation();
  const orbScope = scope ?? attentionScopeForActivity(activity);
  const exitHint = useActivityExitHint(orbScope);
  const visible = useDelayedActivity(active, delayMs, minVisibleMs, exitHint);
  // 渲染层兜底仲裁：同一 scope 已有 Beam 在播放时不渲染 Orb。
  const beamActive = useActiveBeamForScope(orbScope);
  const showOrb = visible && !beamActive;

  useEffect(() => {
    if (!orbScope) return;
    if (showOrb) beginActivity(orbScope);
    else endActivity(orbScope);
  }, [orbScope, showOrb]);

  // 清理：卸载时注销，避免残留导致后续 announce 被判定“有可见 Orb”。
  useEffect(() => {
    if (!orbScope) return;
    return () => endActivity(orbScope);
  }, [orbScope]);

  if (!showOrb && !reserveSpace && fallback === undefined) return null;

  const spec = miraActivitySpec(activity);
  const label = miraActivityLabel(
    activity,
    i18n.resolvedLanguage ?? i18n.language,
  );

  return (
    <span
      className={[
        'mira-inline-activity',
        showOrb ? 'is-visible' : 'is-waiting',
        !showOrb && fallback !== undefined ? 'has-fallback' : null,
        className,
      ].filter(Boolean).join(' ')}
      role={announce && showOrb ? 'status' : undefined}
      aria-hidden={announce && showOrb ? undefined : 'true'}
      aria-label={announce && showOrb ? label : undefined}
    >
      {showOrb ? (
        <ThinkingOrb
          state={spec.state}
          size={spec.size}
          speed={spec.speed}
          theme="auto"
          aria-hidden="true"
        />
      ) : fallback !== undefined ? (
        <span className="mira-inline-activity-fallback" aria-hidden="true">
          {fallback}
        </span>
      ) : undefined}
    </span>
  );
}