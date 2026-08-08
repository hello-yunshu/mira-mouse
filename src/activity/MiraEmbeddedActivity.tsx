// SPDX-License-Identifier: AGPL-3.0-or-later
import { ThinkingOrb } from 'thinking-orbs';
import { useTranslation } from 'react-i18next';
import { miraActivityLabel, miraActivitySpec, type MiraActivityKind } from './activityCatalog';
import { useDelayedActivity } from './useDelayedActivity';

export interface MiraEmbeddedActivityProps {
  active: boolean;
  activity: MiraActivityKind;
  aiAnalysisEnabled?: boolean;
}

/**
 * Activity rendered directly inside an existing surface. It deliberately has
 * no card, blur, shadow, or visible duplicate label: the Orb is the only visual
 * subject, while the localized description remains available to assistive tech.
 */
export function MiraEmbeddedActivity({
  active,
  activity,
  aiAnalysisEnabled = false,
}: MiraEmbeddedActivityProps) {
  const { i18n } = useTranslation();
  const visible = useDelayedActivity(active);

  if (!visible) return null;

  const spec = miraActivitySpec(activity);
  const label = miraActivityLabel(
    activity,
    i18n.resolvedLanguage ?? i18n.language,
    { aiAnalysisEnabled },
  );

  return (
    <div
      className="mira-embedded-activity"
      data-mira-activity={activity}
      role="status"
      aria-live="polite"
      aria-label={label}
    >
      <ThinkingOrb
        state={spec.state}
        size={spec.size}
        speed={spec.speed}
        theme="auto"
        aria-hidden="true"
      />
    </div>
  );
}
