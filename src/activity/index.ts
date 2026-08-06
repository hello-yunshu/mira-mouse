// SPDX-License-Identifier: AGPL-3.0-or-later
import './activity.css';

export { MiraActivityOverlay } from './MiraActivityOverlay';
export {
  MiraInlineActivity,
  type MiraInlineActivityProps,
} from './MiraInlineActivity';
export { useDelayedActivity } from './useDelayedActivity';
export {
  MIRA_ACTIVITY_MIN_VISIBLE_MS,
  MIRA_ACTIVITY_SHOW_DELAY_MS,
  miraActivityLabel,
  miraActivitySpec,
  resolveGlobalMiraActivity,
  type MiraActivityKind,
  type MiraActivitySpec,
  type MiraOrbState,
} from './activityCatalog';
