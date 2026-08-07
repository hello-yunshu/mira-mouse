// SPDX-License-Identifier: AGPL-3.0-or-later

export interface AttentionVisualSupport {
  registeredCustomProperty: boolean;
  colorMix: boolean;
  maskComposite: boolean;
  fullLineBeam: boolean;
}

function supportsRegisteredCustomProperty(): boolean {
  if (typeof CSS === 'undefined') return false;

  const css = CSS as typeof CSS & {
    registerProperty?: (
      definition: {
        name: string;
        syntax?: string;
        inherits: boolean;
        initialValue?: string;
      },
    ) => void;
  };

  return typeof css.registerProperty === 'function';
}

function supportsColorMix(): boolean {
  return (
    typeof CSS !== 'undefined'
    && typeof CSS.supports === 'function'
    && CSS.supports(
      'color',
      'color-mix(in oklch, white 50%, transparent)',
    )
  );
}

function supportsMaskComposite(): boolean {
  if (
    typeof CSS === 'undefined'
    || typeof CSS.supports !== 'function'
  ) {
    return false;
  }

  return (
    CSS.supports(
      'mask-composite',
      'exclude',
    )
    || CSS.supports(
      '-webkit-mask-composite',
      'xor',
    )
  );
}

export function detectAttentionVisualSupport():
  AttentionVisualSupport {
  const registeredCustomProperty =
    supportsRegisteredCustomProperty();

  const colorMix = supportsColorMix();
  const maskComposite = supportsMaskComposite();

  return {
    registeredCustomProperty,
    colorMix,
    maskComposite,
    fullLineBeam:
      registeredCustomProperty
      && colorMix
      && maskComposite,
  };
}