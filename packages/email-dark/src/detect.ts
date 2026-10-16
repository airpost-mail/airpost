// SPDX-License-Identifier: GPL-3.0-only

import { RGBA, relativeLuminance, DARK_LUMINANCE } from './color';

// A painted area of the email: an opaque colour and the area it covers that no painted
// descendant covers. `color: null` marks an area whose lightness is unknown (a background image
// or gradient, or page the email doesn't paint); it is left out of the vote.
export interface PaintedRegion {
  color: RGBA | null;
  area: number;
}

// A run of visible text: its colour, the solid colour it sits on (null when that is unknown,
// as over an image or on page the email doesn't paint) and its length in characters.
export interface TextSample {
  color: RGBA;
  backdrop: RGBA | null;
  weight: number;
}

// Share of the vote that must be dark for the email to count as already dark.
export const ALREADY_DARK_SHARE = 0.5;
// Emails that declare dark-mode support have had their own dark styles applied; they only
// need to be mostly dark to be left alone.
export const DECLARED_DARK_SHARE = 0.25;
// Below this many characters of text the text vote is too thin and painted area decides.
export const MIN_TEXT_WEIGHT = 40;
// Text at least this light was written for a dark background.
const LIGHT_TEXT_LUMINANCE = 0.4;

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

// Share of text written to be read on a dark background: text on a dark backdrop, or light
// text where the backdrop is unknown.
export function textDarkShare(texts: TextSample[]): { share: number; weight: number } {
  let dark = 0;
  let weight = 0;
  for (const { color, backdrop, weight: w } of texts) {
    if (!(w > 0)) continue;
    weight += w;
    const isDark = backdrop
      ? relativeLuminance(backdrop) < DARK_LUMINANCE
      : relativeLuminance(color) >= LIGHT_TEXT_LUMINANCE;
    if (isDark) dark += w;
  }
  return { share: weight > 0 ? dark / weight : 0, weight };
}

/**
 * True when the email was designed dark. The text decides: what most of it was written to be
 * read on. Emails with little text fall back to the share of painted area that is dark.
 * Unpainted page doesn't vote, so a dark 600px table in a wide reading pane still counts.
 */
export function isAlreadyDark(
  regions: PaintedRegion[],
  texts: TextSample[] = [],
  threshold = ALREADY_DARK_SHARE
): boolean {
  const text = textDarkShare(texts);
  if (text.weight >= MIN_TEXT_WEIGHT) return text.share >= threshold;
  return darkShare(regions) >= threshold;
}
