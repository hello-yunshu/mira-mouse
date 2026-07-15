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
  // 允许插件 namespace 回退到 host translation namespace：
  // i18n.t(key, { ns: pluginId }) 找不到时自动查 translation namespace。
  fallbackNS: ['translation'],
  interpolation: { escapeValue: false },
});

/** 加载插件 locale 并注册为 i18n namespace（以插件 ID 命名）。
 * 在应用启动时调用，使插件特定的标签（灯效名、capability 名等）可解析。
 */
export async function loadPluginLocales(): Promise<boolean> {
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    const locales = await invoke<Record<string, Record<string, Record<string, string>>>>('plugin_locales');
    let added = false;
    for (const [pluginId, localeData] of Object.entries(locales)) {
      for (const [lang, dict] of Object.entries(localeData)) {
        // 后端返回扁平 key→value 映射（BTreeMap<String,String>），但 i18next 按 keySeparator '.'
        // 拆分 key 查找嵌套路径。需将扁平 key 转换为嵌套对象后再注册。
        i18n.addResourceBundle(lang, pluginId, unflattenKeys(dict), true, false);
        added = true;
      }
    }
    return added;
  } catch {
    // Tauri 未就绪或无插件 locale：静默跳过，使用 host 回退标签。
    return false;
  }
}

/** 将扁平 key（如 "a.b.c"）转换为嵌套对象（如 {a:{b:{c:value}}}）。
 * i18next 默认按 '.' 拆分 key 查找嵌套资源，因此 addResourceBundle 需要嵌套结构。
 */
function unflattenKeys(flat: Record<string, string>): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(flat)) {
    const parts = key.split('.');
    let current: Record<string, unknown> = result;
    for (let i = 0; i < parts.length - 1; i++) {
      const existing = current[parts[i]];
      if (typeof existing !== 'object' || existing === null || Array.isArray(existing)) {
        current[parts[i]] = {};
      }
      current = current[parts[i]] as Record<string, unknown>;
    }
    current[parts[parts.length - 1]] = value;
  }
  return result;
}

/** 解析 labelKey，优先查找插件 namespace，再回退到 host translation namespace。
 *
 * 早期插件将通用能力写成 `capability.battery`，而宿主词条保存在
 * `plugin.label.capability.battery`。声明式迁移后不能让这类已签名插件
 * 直接把 key 渲染到界面，因此保留这个无厂商知识的通用前缀回退。
 * @param labelKey i18n key（如 "lighting.fixed" 或 "plugin.label.capability.mouse-lighting"）
 * @param pluginId 当前设备匹配的插件 ID（如 "mira.logitech-hidpp"），用于 namespace 查找
 */
export function resolveLabelKey(labelKey: string, pluginId?: string): string {
  if (pluginId && i18n.exists(labelKey, { ns: pluginId })) {
    return i18n.t(labelKey, { ns: pluginId });
  }
  if (i18n.exists(labelKey, { ns: 'translation' })) {
    return i18n.t(labelKey, { ns: 'translation' });
  }
  const prefixedHostKey = `plugin.label.${labelKey}`;
  if (i18n.exists(prefixedHostKey, { ns: 'translation' })) {
    return i18n.t(prefixedHostKey, { ns: 'translation' });
  }
  return labelKey;
}

function findResourcePath(value: unknown, target: string, prefix: string[] = []): string[] | undefined {
  if (typeof value === 'string') return value === target ? prefix : undefined;
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  for (const [key, child] of Object.entries(value)) {
    const found = findResourcePath(child, target, [...prefix, key]);
    if (found) return found;
  }
  return undefined;
}

function readResourcePath(value: unknown, path: string[]): unknown {
  let current = value;
  for (const part of path) {
    if (!current || typeof current !== 'object' || Array.isArray(current)) return undefined;
    current = (current as Record<string, unknown>)[part];
  }
  return current;
}

/** Translate a protocol-provided display string by matching it against the
 * opposite-language locale bundle. This keeps signed plugins compatible when
 * their parser exposes a localized `*Name`/`*Label` alongside the raw value.
 */
export function resolveRuntimeText(value: string, pluginId?: string): string {
  const language: ResolvedLanguage = i18n.resolvedLanguage?.startsWith('en') ? 'en' : 'zh-CN';
  const sourceLanguage: ResolvedLanguage = language === 'en' ? 'zh-CN' : 'en';
  const namespaces = pluginId ? [pluginId, 'translation'] : ['translation'];
  for (const namespace of namespaces) {
    const sourceBundle = i18n.getResourceBundle(sourceLanguage, namespace) as unknown;
    const targetBundle = i18n.getResourceBundle(language, namespace) as unknown;
    const path = findResourcePath(sourceBundle, value);
    if (!path) continue;
    const translated = readResourcePath(targetBundle, path);
    if (typeof translated === 'string') return translated;
  }
  return value;
}

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
