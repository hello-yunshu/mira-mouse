// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, describe, expect, it } from 'vitest';
import i18n, { resolveLabelKey } from './i18n';

afterEach(async () => {
  i18n.removeResourceBundle('zh-CN', 'test-plugin');
  await i18n.changeLanguage('zh-CN');
});

describe('resolveLabelKey', () => {
  it('falls back from declarative capability keys to the host label namespace', async () => {
    await i18n.changeLanguage('zh-CN');
    expect(resolveLabelKey('capability.battery')).toBe('电量');
    expect(resolveLabelKey('lighting.mouse')).toBe('鼠标灯光');

    await i18n.changeLanguage('en');
    expect(resolveLabelKey('capability.battery')).toBe('Battery');
    expect(resolveLabelKey('lighting.receiver')).toBe('Receiver lighting');
  });

  it('keeps a plugin-specific locale ahead of the generic host fallback', async () => {
    await i18n.changeLanguage('zh-CN');
    i18n.addResourceBundle('zh-CN', 'test-plugin', {
      capability: { battery: '插件电量' },
    });
    expect(resolveLabelKey('capability.battery', 'test-plugin')).toBe('插件电量');
  });
});
