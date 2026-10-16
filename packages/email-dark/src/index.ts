// SPDX-License-Identifier: GPL-3.0-only

export {
  type RGBA,
  type HSLA,
  type ColorRole,
  type Poles,
  ColorMapper,
  parseColor,
  formatColor,
  relativeLuminance,
  contrastRatio,
  composite,
  isDarkColor,
  mapColor,
  polesFor,
  DARK_LUMINANCE,
} from './color';
export { type PaintedRegion, darkShare, isAlreadyDark, ALREADY_DARK_SHARE } from './detect';
export {
  type Palette,
  type DarkenOptions,
  type DarkenResult,
  type DarkenStatus,
  darkenDocument,
  removeDarkening,
  isDarkened,
  DEFAULT_MAX_ELEMENTS,
} from './darken';
