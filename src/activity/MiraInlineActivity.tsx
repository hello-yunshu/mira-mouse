// SPDX-License-Identifier: AGPL-3.0-or-later
import { ThinkingOrb } from 'thinking-orbs';
import { useTranslation } from 'react-i18next';
import {
  miraActivityLabel,
  miraActivitySpec,
  type MiraActivityKind,
} from './activityCatalog';
import { useDelayedActivity } from './useDelayedActivity';

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
  /** Reserve the 20px icon slot while idle so button width never changes. */
  reserveSpace?: boolean;
}

export function MiraInlineActivity({
  active,
  activity,
  className,
  announce = false,
  delayMs,
  minVisibleMs,
  reserveSpace = true,
}: MiraInlineActivityProps) {
  const { i18n } = useTranslation();
  const visible = useDelayedActivity(active, delayMs, minVisibleMs);
  const spec = miraActivitySpec(activity);
  const label = miraActivityLabel(
    activity,
    i18n.resolvedLanguage ?? i18n.language,
  );

  if (!active && !visible && !reserveSpace) return null;

  return (
    <span
      className={[
        'mira-inline-activity',
        visible ? 'is-visible' : 'is-waiting',
        className,
      ].filter(Boolean).join(' ')}
      aria-hidden={announce ? undefined : 'true'}
      role={announce && visible ? 'status' : undefined}
      aria-label={announce && visible ? label : undefined}
    >
      {visible && (
        <ThinkingOrb
          state={spec.state}
          size={spec.size}
          speed={spec.speed}
          theme="auto"
          aria-hidden="true"
        />
      )}
    </span>
  );
}
