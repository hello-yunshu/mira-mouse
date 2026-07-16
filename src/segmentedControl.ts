// SPDX-License-Identifier: AGPL-3.0-or-later
import type { CSSProperties } from 'react';

type SegmentedIndicatorStyle = CSSProperties & {
  '--segmented-indicator-accent'?: string;
  '--segmented-indicator-inset'?: string;
  '--segmented-indicator-left'?: string;
  '--segmented-indicator-width'?: string;
};

export function segmentedIndicatorStyle(
  itemCount: number,
  activeIndex: number,
  options: { accent?: string; gap?: number; padding?: number } = {},
): SegmentedIndicatorStyle {
  const count = Math.max(1, Math.floor(itemCount));
  const index = Math.min(Math.max(0, Math.floor(activeIndex)), count - 1);
  const gap = options.gap ?? 3;
  const padding = options.padding ?? 3;
  const totalInset = padding * 2 + gap * (count - 1);
  const itemPercent = 100 / count;
  const tidy = (value: number) => Number(value.toFixed(6));
  const indicatorWidth = `calc(${tidy(itemPercent)}% - ${tidy(totalInset / count)}px)`;
  const indicatorLeft = `calc(${tidy(index * itemPercent)}% + ${tidy(padding + index * (gap - totalInset / count))}px)`;

  return {
    '--segmented-indicator-accent': options.accent,
    '--segmented-indicator-inset': `${padding}px`,
    '--segmented-indicator-left': indicatorLeft,
    '--segmented-indicator-width': indicatorWidth,
  };
}
