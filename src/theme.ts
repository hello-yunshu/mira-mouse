// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ThemeMode } from './types';
import { setTheme } from '@tauri-apps/api/app';

export const DEFAULT_THEME_ACCENT = '#ffb3b3';

function colorHueAndChroma(color: string): { hue: number; chroma: number; lightness: number } | undefined {
  if (!/^#[0-9a-f]{6}$/i.test(color)) return undefined;
  const r = Number.parseInt(color.slice(1, 3), 16) / 255;
  const g = Number.parseInt(color.slice(3, 5), 16) / 255;
  const b = Number.parseInt(color.slice(5, 7), 16) / 255;
  const hue = Math.round((Math.atan2(Math.sqrt(3) * (g - b), 2 * r - g - b) * 180) / Math.PI + 360) % 360;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  // 感知亮度（Rec.601），用于暗色模式下判断原色是否偏亮
  const lightness = 0.299 * r + 0.587 * g + 0.114 * b;
  return { hue, chroma: max - min, lightness };
}

export function themeAccent(color?: string, isDark = false): string {
  if (!color || !/^#[0-9a-f]{6}$/i.test(color)) return DEFAULT_THEME_ACCENT;
  if (color.toLowerCase() === DEFAULT_THEME_ACCENT) return DEFAULT_THEME_ACCENT;
  const parsed = colorHueAndChroma(color);
  if (!parsed) return DEFAULT_THEME_ACCENT;
  const { hue, chroma, lightness } = parsed;
  const capped = hue < 25 || hue > 315 ? Math.min(chroma * 0.08, 0.055) : Math.min(chroma * 0.11, 0.075);
  // 亮色模式：固定 72%，不把暗色灯光再压低；
  // 暗色模式：原色越亮压越多，下限 56% 保证在暗背景上仍可见。
  let L = 72;
  if (isDark) {
    L = Math.round(Math.max(56, Math.min(64, 64 - (lightness - 0.5) * 18)) * 10) / 10;
  }
  return `oklch(${L}% ${capped.toFixed(3)} ${hue})`;
}

export function pastelDisplayColor(color?: string, fallback = DEFAULT_THEME_ACCENT): string {
  if (!color || !/^#[0-9a-f]{6}$/i.test(color)) return fallback;
  if (color.toLowerCase() === DEFAULT_THEME_ACCENT) return DEFAULT_THEME_ACCENT;
  const parsed = colorHueAndChroma(color);
  if (!parsed) return fallback;
  const { hue, chroma } = parsed;
  const limit = hue < 25 || hue > 315 ? 0.058 : 0.082;
  const softened = chroma > 0.02 ? Math.max(chroma * 0.28, 0.026) : chroma * 0.28;
  const capped = Math.min(softened, limit);
  return `oklch(78% ${capped.toFixed(3)} ${hue})`;
}

export function applyTheme(mode: ThemeMode, accent?: string): void {
  lastMode = mode;
  lastAccent = accent;
  document.documentElement.dataset.theme = mode;
  document.documentElement.style.setProperty('--accent', themeAccent(accent, resolveDark(mode)));
  void setTheme(mode === 'system' ? null : mode).catch(() => {});
  if (mode === 'system') ensureSystemListener();
}

function resolveDark(mode: ThemeMode): boolean {
  if (mode === 'dark') return true;
  if (mode === 'light') return false;
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false;
  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

let lastMode: ThemeMode | null = null;
let lastAccent: string | undefined;
let systemListenerInstalled = false;

/** system 模式下监听 OS 亮暗切换，自动重算 accent 亮度。 */
function ensureSystemListener(): void {
  if (systemListenerInstalled) return;
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
  const mql = window.matchMedia('(prefers-color-scheme: dark)');
  mql.addEventListener('change', () => {
    if (lastMode === 'system') applyTheme('system', lastAccent);
  });
  systemListenerInstalled = true;
}
