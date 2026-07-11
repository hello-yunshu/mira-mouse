// SPDX-License-Identifier: AGPL-3.0-or-later
import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { BatteryLevelIcon } from './BatteryLevelIcon';

describe('BatteryLevelIcon', () => {
  it('renders a charging bolt in the same current color as the battery icon', () => {
    const { container } = render(<BatteryLevelIcon percentage={80} charging />);
    expect(container.querySelector('.battery-level-bolt')).toHaveAttribute('fill', 'currentColor');
  });
});
