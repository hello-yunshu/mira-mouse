// SPDX-License-Identifier: AGPL-3.0-or-later
// 克制型 Attention Beam 的统一出口。

import './attention-beam.css';

export { AttentionBeamLayer, type AttentionBeamLayerProps } from './AttentionBeamLayer';
export { AttentionSurface, type AttentionSurfaceProps } from './AttentionSurface';
export { LightingAttention } from './LightingAttention';
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
  MAX_ATTENTION_QUEUE,
  DEVICE_READY_STARTUP_GRACE_MS,
  ATTENTION_PRIORITY,
  attentionLightingKey,
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
  type AttentionBeamRequest,
  type AttentionBeamVariant,
  type MutationResult,
  type DeviceAttentionContext,
  type DeviceAttentionOutcome,
} from './attentionTypes';