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

describe('donate aura static rounded clipping', () => {
  it('keeps the original fixed cloud on a hard rounded clip', () => {
    const aura = rule('.donate-aura');
    expect(aura).toMatch(/overflow:\s*hidden/);
    expect(aura).toMatch(/border-radius:\s*inherit/);
    expect(aura).toMatch(/clip-path:\s*inset\(0 round 15px\)/);

    const cloud = rule('.donate-aura::before');
    expect(cloud).not.toMatch(/filter\s*:/);
    expect(cloud).not.toMatch(/animation\s*:/);
    expect(cloud).toMatch(/radial-gradient/);
    expect(cloud).toMatch(/top:\s*-45%/);
    expect(cloud).toMatch(/right:\s*-18%/);
    expect(cloud).toMatch(/width:\s*72%/);
    expect(cloud).toMatch(/height:\s*190%/);
    expect(cloud).toMatch(/circle at 32% 32%/);
    expect(cloud).toMatch(/circle at 70% 68%/);
  });
});
