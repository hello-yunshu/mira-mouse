// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import { segmentedIndicatorStyle } from './segmentedControl';

describe('segmentedIndicatorStyle', () => {
  it('positions a two-item indicator inside shared padding and gap', () => {
    expect(segmentedIndicatorStyle(2, 1)).toMatchObject({
      '--segmented-indicator-inset': '3px',
      '--segmented-indicator-left': 'calc(50% + 1.5px)',
      '--segmented-indicator-width': 'calc(50% - 4.5px)',
    });
  });

  it('supports wider segmented controls and clamps the active index', () => {
    expect(segmentedIndicatorStyle(3, 9, { gap: 3, padding: 4 })).toMatchObject({
      '--segmented-indicator-inset': '4px',
      '--segmented-indicator-left': 'calc(66.666667% + 0.666667px)',
      '--segmented-indicator-width': 'calc(33.333333% - 4.666667px)',
    });
  });
});
