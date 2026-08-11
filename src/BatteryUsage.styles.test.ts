// SPDX-License-Identifier: AGPL-3.0-or-later
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const styles = readFileSync(join(import.meta.dirname, 'styles.css'), 'utf8');

function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = styles.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  expect(match, `missing CSS rule for ${selector}`).not.toBeNull();
  return match?.[1] ?? '';
}

describe('battery usage transition geometry', () => {
  it('keeps multiline fade layers on the final text-column width', () => {
    const multiline = rule('.battery-fade-multiline');
    expect(multiline).toMatch(/width:\s*100%/);
    expect(multiline).toMatch(/min-width:\s*0/);

    const body = rule('.battery-insight-card .insight-body');
    expect(body).toMatch(/flex:\s*1 1 0/);
    expect(body).toMatch(/min-width:\s*0/);

    const tooltip = rule('.battery-insight-card .insight-body > .tooltip');
    expect(tooltip).toMatch(/align-self:\s*stretch/);
    expect(tooltip).toMatch(/min-width:\s*0/);
  });
});
