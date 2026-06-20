// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import { themeAccent } from './theme';
describe('dynamic accent', () => {
  it('falls back for missing and malformed colors', () => expect(themeAccent('red')).toBe('#D8B0B7'));
  it('keeps the default accent exact', () => expect(themeAccent('#d8b0b7')).toBe('#D8B0B7'));
  it('caps saturated red more strictly', () => expect(themeAccent('#ff0000')).toContain('0.055'));
});
