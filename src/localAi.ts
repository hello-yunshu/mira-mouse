// SPDX-License-Identifier: AGPL-3.0-or-later
import type { AppSettings } from './types';

export const LOCAL_AI_FEATURE = {
  batteryUsage: 'batteryUsage',
} as const;

export type LocalAiFeatureId = (typeof LOCAL_AI_FEATURE)[keyof typeof LOCAL_AI_FEATURE];
export type LocalAiFeatures = Record<string, boolean>;

// Battery usage is the only current consumer, so existing installs keep their
// current behavior. Future consumers must opt in with their own feature ID.
export const DEFAULT_LOCAL_AI_FEATURES: LocalAiFeatures = {
  [LOCAL_AI_FEATURE.batteryUsage]: true,
};

export function localAiFeatureSelected(
  features: LocalAiFeatures | undefined,
  feature: LocalAiFeatureId,
): boolean {
  return features?.[feature] ?? DEFAULT_LOCAL_AI_FEATURES[feature] ?? false;
}

export function localAiFeatureEnabled(
  settings: Pick<AppSettings, 'localAiAnalysisEnabled' | 'localAiFeatures'>,
  feature: LocalAiFeatureId,
): boolean {
  return settings.localAiAnalysisEnabled && localAiFeatureSelected(settings.localAiFeatures, feature);
}

export function setLocalAiFeature(
  features: LocalAiFeatures | undefined,
  feature: LocalAiFeatureId,
  enabled: boolean,
): LocalAiFeatures {
  return {
    ...DEFAULT_LOCAL_AI_FEATURES,
    ...features,
    [feature]: enabled,
  };
}
