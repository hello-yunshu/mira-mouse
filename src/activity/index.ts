// SPDX-License-Identifier: AGPL-3.0-or-later
import './activity.css';

export { MiraActivityOverlay } from './MiraActivityOverlay';
export {
  MiraActivityButton,
  type MiraActivityButtonProps,
} from './MiraActivityButton';
export {
  MiraEmbeddedActivity,
  type MiraEmbeddedActivityProps,
} from './MiraEmbeddedActivity';
export {
  MiraInlineActivity,
  type MiraInlineActivityProps,
} from './MiraInlineActivity';
export { useDelayedActivity } from './useDelayedActivity';
export {
  attentionScopeForActivity,
  announceAfterOrbExit,
  isActivityVisible,
  waitForActivityExit,
  type ActivityScope,
} from './activityCoordinator';
export {
  MIRA_ACTIVITY_MIN_VISIBLE_MS,
  MIRA_ACTIVITY_SHOW_DELAY_MS,
  miraActivityLabel,
  miraActivitySpec,
  resolveGlobalMiraActivity,
  type MiraActivityKind,
  type MiraActivitySpec,
  type MiraGlobalActivity,
  type MiraOrbState,
} from './activityCatalog';
