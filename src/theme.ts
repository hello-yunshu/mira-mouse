// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ThemeMode } from './types';

export function themeAccent(color?: string): string {
  if (!color || !/^#[0-9a-f]{6}$/i.test(color)) return 'oklch(72% 0.025 285)';
  const r = Number.parseInt(color.slice(1, 3), 16) / 255;
  const g = Number.parseInt(color.slice(3, 5), 16) / 255;
  const b = Number.parseInt(color.slice(5, 7), 16) / 255;
  const hue = Math.round((Math.atan2(Math.sqrt(3) * (g - b), 2 * r - g - b) * 180) / Math.PI + 360) % 360;
  const max = Math.max(r, g, b); const min = Math.min(r, g, b);
  const chroma = max - min;
  const capped = hue < 25 || hue > 315 ? Math.min(chroma * 0.08, 0.055) : Math.min(chroma * 0.11, 0.075);
  return `oklch(72% ${capped.toFixed(3)} ${hue})`;
}

export function applyTheme(mode: ThemeMode, accent?: string): void {
  document.documentElement.dataset.theme = mode;
  document.documentElement.style.setProperty('--accent', themeAccent(accent));
}

