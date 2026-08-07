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
  it('uses large global orbs and restrained inline orbs', () => {
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

  it('prioritizes an unresolved battery modal over the covered dashboard', () => {
    document.body.innerHTML = `
      <main class="dashboard is-initializing"></main>
      <div class="battery-usage-modal"></div>
    `;
    expect(resolveGlobalMiraActivity(document)).toBe('battery-analysis');
  });

  it('stops when battery content or an empty state is ready', () => {
    document.body.innerHTML = `
      <div class="battery-usage-modal">
        <div class="battery-status-strip-shell"></div>
      </div>
    `;
    expect(resolveGlobalMiraActivity(document)).toBeNull();

    document.body.innerHTML = `
      <div class="battery-usage-modal">
        <div class="battery-usage-empty"></div>
      </div>
    `;
    expect(resolveGlobalMiraActivity(document)).toBeNull();
  });
});
