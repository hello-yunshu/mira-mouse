// SPDX-License-Identifier: AGPL-3.0-or-later
// 克制型 Attention Beam 的统一出口。

import './attention-beam.css';

export { AttentionBeamLayer, type AttentionBeamLayerProps } from './AttentionBeamLayer';
export { AttentionSurface, type AttentionSurfaceProps } from './AttentionSurface';
export { useAttentionFeedback, type AttentionBehavior } from './useAttentionFeedback';
export {
  announceAttentionRequest,
  finishActiveAttentionRequest,
  getAttentionBusState,
  onAttentionBusStateChange,
  hasAttentionEventPlayedOnce,
  attentionBusReduce,
  createInitialAttentionBusState,
  resetAttentionBusForTests,
  type AttentionBusState,
} from './attentionCore';
export {
  registerLightingAttention,
  peekPendingLightingAttention,
  takePendingLightingAttention,
  clearPendingLightingAttention,
  confirmPendingLightingAttention,
  resetPendingLightingAttentionForTests,
  attentionLightingMutationKey,
  lightingAttentionRequest,
  type LightingAttentionKind,
  type LightingKeyKind,
  type PendingLightingAttention,
  type ZoneLightingState,
} from './attentionLighting';
export {
  MAX_ATTENTION_QUEUE,
  DEVICE_READY_STARTUP_GRACE_MS,
  ATTENTION_PRIORITY,
  attentionAppUpdateKey,
  attentionAppRestartKey,
  attentionPluginUpdateKey,
  attentionPluginInstalledKey,
  attentionLocalAiUpdateKey,
  attentionLocalAiInstalledKey,
  attentionDeviceKey,
  attentionColorForZone,
  attentionAccentColor,
  attentionDesaturatedAccent,
  attentionNeutralColor,
  attentionIsDarkTheme,
  reduceDeviceAttention,
  reduceDeviceAttentionByIdentity,
  resolveUpdateAttentionTarget,
  type AttentionBeamRequest,
  type AttentionBeamVariant,
  type DeviceAttentionContext,
  type DeviceAttentionOutcome,
  type AttentionView,
  type UpdateAttentionTarget,
  type UpdateAttentionKind,
  type UpdateAttentionContext,
} from './attentionTypes';
