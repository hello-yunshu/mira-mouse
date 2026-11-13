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

describe('Windows top navigation', () => {
  it('reserves the same transparent border when device/settings changes active state', () => {
    const activeRule = rule('.platform-windows .nav-link.active:not(.nav-about)');
    expect(activeRule).toMatch(/background:\s*transparent/);
    expect(activeRule).toMatch(/border:\s*1px solid transparent/);
  });

  it('uses only the themed about icon for the active state', () => {
    const activeAboutRule = rule('.platform-windows .nav-link.nav-about.active');
    expect(activeAboutRule).toMatch(/color:\s*var\(--accent\)/);
    expect(activeAboutRule).toMatch(/background:\s*transparent/);
    expect(activeAboutRule).toMatch(/border:\s*1px solid transparent/);
    expect(activeAboutRule).toMatch(/outline:\s*none/);
  });

  it('keeps the about icon transparent while hovered', () => {
    const hoverAboutRule = rule('.platform-windows .nav-link.nav-about:hover');
    expect(hoverAboutRule).toMatch(/background:\s*transparent/);
  });
});
