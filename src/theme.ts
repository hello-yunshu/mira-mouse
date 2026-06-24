// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ThemeMode } from './types';
import { setTheme } from '@tauri-apps/api/app';

function colorHueAndChroma(color: string): { hue: number; chroma: number } | undefined {
  if (!/^#[0-9a-f]{6}$/i.test(color)) return undefined;
  const r = Number.parseInt(color.slice(1, 3), 16) / 255;
  const g = Number.parseInt(color.slice(3, 5), 16) / 255;
  const b = Number.parseInt(color.slice(5, 7), 16) / 255;
  const hue = Math.round((Math.atan2(Math.sqrt(3) * (g - b), 2 * r - g - b) * 180) / Math.PI + 360) % 360;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  return { hue, chroma: max - min };
}

export function themeAccent(color?: string): string {
  if (!color || !/^#[0-9a-f]{6}$/i.test(color)) return '#D8B0B7';
  if (color.toUpperCase() === '#D8B0B7') return '#D8B0B7';
  const parsed = colorHueAndChroma(color);
  if (!parsed) return '#D8B0B7';
  const { hue, chroma } = parsed;
  const capped = hue < 25 || hue > 315 ? Math.min(chroma * 0.08, 0.055) : Math.min(chroma * 0.11, 0.075);
  return `oklch(72% ${capped.toFixed(3)} ${hue})`;
}

export function pastelDisplayColor(color?: string, fallback = '#D8B0B7'): string {
  if (!color || !/^#[0-9a-f]{6}$/i.test(color)) return fallback;
  if (color.toUpperCase() === '#D8B0B7') return '#D8B0B7';
  const parsed = colorHueAndChroma(color);
  if (!parsed) return fallback;
  const { hue, chroma } = parsed;
  const limit = hue < 25 || hue > 315 ? 0.058 : 0.082;
  const softened = chroma > 0.02 ? Math.max(chroma * 0.28, 0.026) : chroma * 0.28;
  const capped = Math.min(softened, limit);
  return `oklch(78% ${capped.toFixed(3)} ${hue})`;
}

export function applyTheme(mode: ThemeMode, accent?: string): void {
  document.documentElement.dataset.theme = mode;
  document.documentElement.style.setProperty('--accent', themeAccent(accent));
  void setTheme(mode === 'system' ? null : mode).catch(() => {});
}
