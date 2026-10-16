// SPDX-License-Identifier: GPL-3.0-only

import { RGBA, relativeLuminance, DARK_LUMINANCE } from './color';

// A painted area of the email: an opaque colour and the area it covers that no painted
// descendant covers. `color: null` marks a background image or gradient, whose lightness is
// unknown; it is left out of the vote.
export interface PaintedRegion {
  color: RGBA | null;
  area: number;
}

// Share of the (known) painted area that must be dark for the email to count as already dark.
export const ALREADY_DARK_SHARE = 0.5;

export function darkShare(regions: PaintedRegion[]): number {
  let dark = 0;
  let total = 0;
  for (const { color, area } of regions) {
    if (!color || !(area > 0)) continue;
    total += area;
    if (relativeLuminance(color) < DARK_LUMINANCE) dark += area;
  }
  return total > 0 ? dark / total : 0;
}

// True when the email was designed dark: most of what it paints, counting unpainted page as
// the page colour the email assumes, is dark.
export function isAlreadyDark(regions: PaintedRegion[]): boolean {
  return darkShare(regions) >= ALREADY_DARK_SHARE;
}
