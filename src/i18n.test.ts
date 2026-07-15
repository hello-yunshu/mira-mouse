// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, describe, expect, it } from 'vitest';
import i18n, { resolveLabelKey, resolveRuntimeText } from './i18n';

afterEach(async () => {
  i18n.removeResourceBundle('zh-CN', 'test-plugin');
  i18n.removeResourceBundle('en', 'test-plugin');
  await i18n.changeLanguage('zh-CN');
});

describe('resolveRuntimeText', () => {
  it('maps a parser-provided localized display value to the active language', async () => {
    i18n.addResourceBundle('zh-CN', 'test-plugin', { effect: { breathing: '呼吸' } });
    i18n.addResourceBundle('en', 'test-plugin', { effect: { breathing: 'Breathing' } });

    await i18n.changeLanguage('en');
    expect(resolveRuntimeText('呼吸', 'test-plugin')).toBe('Breathing');
    expect(resolveRuntimeText('设备自定义名称', 'test-plugin')).toBe('设备自定义名称');
  });
});

describe('resolveLabelKey', () => {
  it('falls back from declarative capability keys to the host label namespace', async () => {
    await i18n.changeLanguage('zh-CN');
    expect(resolveLabelKey('capability.battery')).toBe('电量');
    expect(resolveLabelKey('lighting.mouse')).toBe('鼠标灯光');

    await i18n.changeLanguage('en');
    expect(resolveLabelKey('capability.battery')).toBe('Battery');
    expect(resolveLabelKey('capability.polling-rate')).toBe('Polling');
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

describe('English locale coverage', () => {
  it('does not contain Chinese UI text outside the native language name', () => {
    const bundle = i18n.getResourceBundle('en', 'translation') as Record<string, unknown>;
    const chinesePaths: string[] = [];
    const visit = (value: unknown, path = '') => {
      if (typeof value === 'string') {
        if (/\p{Script=Han}/u.test(value)) chinesePaths.push(path);
        return;
      }
      if (!value || typeof value !== 'object' || Array.isArray(value)) return;
      for (const [key, child] of Object.entries(value)) visit(child, path ? `${path}.${key}` : key);
    };
    visit(bundle);
    expect(chinesePaths).toEqual(['settings.language.zhCN']);
  });
});
