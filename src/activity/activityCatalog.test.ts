// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, describe, expect, it } from 'vitest';
import {
  miraActivityLabel,
  miraActivitySpec,
  resolveGlobalMiraActivity,
} from './activityCatalog';

afterEach(() => {
  document.body.innerHTML = '';
});

describe('activity catalog', () => {
  it('distinguishes global, embedded, and restrained inline orbs', () => {
    expect(miraActivitySpec('device-initializing')).toMatchObject({
      state: 'connecting',
      size: 64,
      layer: 'global',
    });
    expect(miraActivitySpec('exporting-battery-history')).toMatchObject({
      state: 'composing',
      size: 20,
      layer: 'inline',
    });
    expect(miraActivitySpec('battery-analysis')).toMatchObject({
      state: 'solving',
      size: 64,
      layer: 'embedded',
    });
  });

  it('provides bilingual semantic labels', () => {
    expect(miraActivityLabel('scanning-devices', 'zh-CN')).toBe('正在扫描设备…');
    expect(miraActivityLabel('checking-app-update', 'en')).toBe(
      'Checking for application updates…',
    );
  });
});

describe('resolveGlobalMiraActivity', () => {
  it('detects device initialization', () => {
    document.body.innerHTML = '<main class="dashboard is-initializing"></main>';
    expect(resolveGlobalMiraActivity(document)).toBe('device-initializing');
  });

  it('keeps the global detector device-only when a battery modal is open', () => {
    document.body.innerHTML = `
      <main class="dashboard is-initializing"></main>
      <div class="battery-usage-modal"></div>
    `;
    expect(resolveGlobalMiraActivity(document)).toBe('device-initializing');
  });

  it('never promotes a battery modal into a global glass activity card', () => {
    document.body.innerHTML = '<div class="battery-usage-modal"></div>';
    expect(resolveGlobalMiraActivity(document)).toBeNull();
  });
});
