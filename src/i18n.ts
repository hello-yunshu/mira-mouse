// SPDX-License-Identifier: AGPL-3.0-or-later
import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import zhCN from './locales/zh-CN.json';
import en from './locales/en.json';

export type AppLanguage = 'auto' | 'zh-CN' | 'en';
export type ResolvedLanguage = 'zh-CN' | 'en';

/** Resolve a settings language value to a concrete supported language. */
export function resolveLanguage(lang: AppLanguage): ResolvedLanguage {
  if (lang === 'zh-CN') return 'zh-CN';
  if (lang === 'en') return 'en';
  // auto: follow system language; default to Chinese when undetectable.
  const nav = (typeof navigator !== 'undefined' ? navigator.language : '') || '';
  const lower = nav.toLowerCase();
  if (lower.startsWith('en')) return 'en';
  if (lower.startsWith('zh')) return 'zh-CN';
  return 'zh-CN';
}

i18n.use(initReactI18next).init({
  resources: {
    'zh-CN': { translation: zhCN },
    en: { translation: en },
  },
  lng: resolveLanguage('auto'),
  fallbackLng: 'zh-CN',
  interpolation: { escapeValue: false },
});

/** Apply a settings language value: switch i18n language and update <html lang>. */
export function applyLanguage(lang: AppLanguage): void {
  const resolved = resolveLanguage(lang);
  void i18n.changeLanguage(resolved);
  if (typeof document !== 'undefined') {
    document.documentElement.lang = resolved;
  }
}

if (typeof document !== 'undefined') {
  document.documentElement.lang = resolveLanguage('auto');
}

export default i18n;
