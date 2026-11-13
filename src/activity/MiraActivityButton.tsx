// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { MiraInlineActivity } from './MiraInlineActivity';
import type { MiraActivityKind } from './activityCatalog';

export interface MiraActivityButtonProps
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'> {
  active: boolean;
  activity: MiraActivityKind;
  children: ReactNode;
  leading?: ReactNode;
  announce?: boolean;
  delayMs?: number;
}

/**
 * Button contract for a single, unambiguous process state:
 *
 * - 0–300ms: keep the original icon/text, so quick tasks do not flash.
 * - Orb visible: replace the entire visual label with one centered Orb.
 * - Done: restore the unchanged label and geometry.
 *
 * The label stays in the DOM while concealed, preserving button width and its
 * accessible name. `data-mira-processing` separates a busy button from an
 * unavailable disabled button so the Orb is not dimmed by generic disabled UI.
 */
export function MiraActivityButton({
  active,
  activity,
  children,
  leading,
  announce = false,
  delayMs,
  className,
  disabled,
  ...buttonProps
}: MiraActivityButtonProps) {
  return (
    <button
      {...buttonProps}
      className={['mira-activity-button', className].filter(Boolean).join(' ')}
      data-mira-processing={active ? 'true' : undefined}
      aria-busy={active || undefined}
      disabled={disabled || active}
    >
      <MiraInlineActivity
        active={active}
        activity={activity}
        delayMs={delayMs}
        announce={announce}
        layout="overlay"
        label={(
          <>
            {leading}
            {children}
          </>
        )}
      />
    </button>
  );
}
