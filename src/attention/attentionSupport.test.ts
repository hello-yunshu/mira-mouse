// SPDX-License-Identifier: AGPL-3.0-or-later

import { afterEach, describe, expect, it, vi } from 'vitest';
import { detectAttentionVisualSupport, type AttentionVisualSupport } from './attentionSupport';

type CssLike = {
  registerProperty?: (...args: unknown[]) => void;
  supports?: (...args: unknown[]) => boolean;
  [key: string]: unknown;
};

function supportsReturning(property: unknown, value?: unknown): boolean {
  if (property === 'color') {
    return typeof value === 'string' && value.startsWith('color-mix(');
  }
  return property === 'mask-composite' || property === '-webkit-mask-composite';
}

const fullSupport: AttentionVisualSupport = {
  registeredCustomProperty: true,
  colorMix: true,
  maskComposite: true,
  fullLineBeam: true,
};

function stubCss(css: CssLike | undefined): void {
  vi.stubGlobal('CSS', css);
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('detectAttentionVisualSupport', () => {
  it('all supported → fullLineBeam=true', () => {
    stubCss({
      registerProperty: () => {},
      supports: supportsReturning,
    });
    const support = detectAttentionVisualSupport();
    expect(support.registeredCustomProperty).toBe(true);
    expect(support.colorMix).toBe(true);
    expect(support.maskComposite).toBe(true);
    expect(support.fullLineBeam).toBe(true);
    expect(support).toEqual(fullSupport);
  });

  it('no registerProperty → fullLineBeam=false', () => {
    stubCss({
      supports: () => true,
    });
    const support = detectAttentionVisualSupport();
    expect(support.registeredCustomProperty).toBe(false);
    expect(support.colorMix).toBe(true);
    expect(support.maskComposite).toBe(true);
    expect(support.fullLineBeam).toBe(false);
  });

  it('no colorMix → fullLineBeam=false', () => {
    stubCss({
      registerProperty: () => {},
      supports: (property: unknown) => (
        property === 'mask-composite' || property === '-webkit-mask-composite'
      ),
    });
    const support = detectAttentionVisualSupport();
    expect(support.registeredCustomProperty).toBe(true);
    expect(support.colorMix).toBe(false);
    expect(support.maskComposite).toBe(true);
    expect(support.fullLineBeam).toBe(false);
  });

  it('no mask → fullLineBeam=false', () => {
    stubCss({
      registerProperty: () => {},
      supports: (property: unknown, value: unknown) => (
        property === 'color'
        && typeof value === 'string'
        && value.startsWith('color-mix(')
      ),
    });
    const support = detectAttentionVisualSupport();
    expect(support.registeredCustomProperty).toBe(true);
    expect(support.colorMix).toBe(true);
    expect(support.maskComposite).toBe(false);
    expect(support.fullLineBeam).toBe(false);
  });

  it('CSS global unavailable → all false, no throw', () => {
    stubCss(undefined);
    expect(() => detectAttentionVisualSupport()).not.toThrow();
    const support = detectAttentionVisualSupport();
    expect(support).toEqual({
      registeredCustomProperty: false,
      colorMix: false,
      maskComposite: false,
      fullLineBeam: false,
    });
  });
});