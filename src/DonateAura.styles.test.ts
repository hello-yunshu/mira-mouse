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
  it('keeps the blurred diagonal cloud on a hard rounded clip', () => {
    const aura = rule('.donate-aura');
    expect(aura).toMatch(/overflow:\s*hidden/);
    expect(aura).toMatch(/border-radius:\s*inherit/);
    expect(aura).toMatch(/clip-path:\s*inset\(0 round 15px\)/);

    const cloud = rule('.donate-aura::before');
    // 云团保持静态：blur + mask 双保险防截断线，绝无动画。
    expect(cloud).not.toMatch(/animation\s*:/);
    expect(cloud).toMatch(/radial-gradient/);
    expect(cloud).toMatch(/filter:\s*blur\(22px\)/);
    expect(cloud).toMatch(/mask-image:\s*linear-gradient\(to right, transparent, #fff 14%\)/);
    expect(cloud).toMatch(/top:\s*-45%/);
    expect(cloud).toMatch(/right:\s*-18%/);
    expect(cloud).toMatch(/width:\s*72%/);
    expect(cloud).toMatch(/height:\s*190%/);
    // 蓝色先验峰压右下角、绿色向左下弥散：右下 → 左下对角线。
    expect(cloud).toMatch(/circle at 61% 62%/);
    expect(cloud).toMatch(/circle at 20% 80%/);
  });
});
