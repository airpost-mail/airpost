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
export {
  type PaintedRegion,
  type TextSample,
  darkShare,
  textDarkShare,
  isAlreadyDark,
  ALREADY_DARK_SHARE,
  DECLARED_DARK_SHARE,
  MIN_TEXT_WEIGHT,
} from './detect';
export {
  type ColorScheme,
  applyColorScheme,
  declaredPageColor,
  declaresDarkSupport,
} from './scheme';
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
