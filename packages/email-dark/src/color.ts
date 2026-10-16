// SPDX-License-Identifier: GPL-3.0-only
//
// Colour parsing and the dark-scheme colour mapping.
//
// `rgbToHsl`, `hslToRgb`, `scale` and the bodies of `darkBackground`, `lightForeground` and
// `darkBorder` are ported from Dark Reader's dynamic mode
// (https://github.com/darkreader/darkreader, `src/utils/color.ts`, `src/utils/math.ts` and
// `src/inject/dynamic-theme/modify-colors.ts`: `modifyBgHSL`, `modifyFgHSL`, `modifyBorderHSL`).
// Dark Reader is MIT licensed, Copyright (c) 2026 Dark Reader Ltd.; see NOTICE.md at the
// repository root for the licence text.

export interface RGBA {
  r: number;
  g: number;
  b: number;
  a: number;
}

export interface HSLA {
  h: number;
  s: number;
  l: number;
  a: number;
}

export function scale(x: number, inLow: number, inHigh: number, outLow: number, outHigh: number) {
  return ((x - inLow) * (outHigh - outLow)) / (inHigh - inLow) + outLow;
}

function clamp(x: number, min: number, max: number) {
  return Math.min(max, Math.max(min, x));
}

const RGB_RE = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:\s*[,/]\s*([\d.]+)(%?))?\s*\)$/i;
const SRGB_RE =
  /^color\(srgb\s+([\d.e-]+)\s+([\d.e-]+)\s+([\d.e-]+)(?:\s*\/\s*([\d.]+)(%?))?\s*\)$/i;
const HEX_RE = /^#([\da-f]{3,8})$/i;

function alpha(value: string | undefined, percent: string | undefined) {
  if (value === undefined) return 1;
  const n = Number(value);
  return clamp(percent ? n / 100 : n, 0, 1);
}

// Parses the forms Chromium uses for computed colours (`rgb()`, `rgba()`, `color(srgb …)`),
// hex, and `transparent`. Anything else (named colours, `oklch()`, `currentcolor`) returns null
// and is left untouched by callers.
export function parseColor(value: string | null | undefined): RGBA | null {
  if (!value) return null;
  const v = value.trim();
  if (v === 'transparent') return { r: 0, g: 0, b: 0, a: 0 };

  let m = RGB_RE.exec(v);
  if (m) {
    return { r: Number(m[1]), g: Number(m[2]), b: Number(m[3]), a: alpha(m[4], m[5]) };
  }
  m = SRGB_RE.exec(v);
  if (m) {
    const ch = (s: string) => Math.round(clamp(Number(s), 0, 1) * 255);
    return { r: ch(m[1]), g: ch(m[2]), b: ch(m[3]), a: alpha(m[4], m[5]) };
  }
  m = HEX_RE.exec(v);
  if (m) {
    let hex = m[1];
    if (hex.length === 3 || hex.length === 4) {
      hex = hex
        .split('')
        .map((c) => c + c)
        .join('');
    }
    if (hex.length !== 6 && hex.length !== 8) return null;
    const n = (i: number) => parseInt(hex.slice(i, i + 2), 16);
    return { r: n(0), g: n(2), b: n(4), a: hex.length === 8 ? n(6) / 255 : 1 };
  }
  return null;
}

// Same serialisation as Chromium's computed style, so formatted values can be compared with
// `getComputedStyle` output.
export function formatColor({ r, g, b, a }: RGBA): string {
  const R = Math.round(r);
  const G = Math.round(g);
  const B = Math.round(b);
  if (a >= 1) return `rgb(${R}, ${G}, ${B})`;
  return `rgba(${R}, ${G}, ${B}, ${Math.round(a * 1000) / 1000})`;
}

export function rgbToHsl({ r: r255, g: g255, b: b255, a }: RGBA): HSLA {
  const r = r255 / 255;
  const g = g255 / 255;
  const b = b255 / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const c = max - min;
  const l = (max + min) / 2;
  if (c === 0) return { h: 0, s: 0, l, a };

  let h = (max === r ? ((g - b) / c) % 6 : max === g ? (b - r) / c + 2 : (r - g) / c + 4) * 60;
  if (h < 0) h += 360;
  const s = c / (1 - Math.abs(2 * l - 1));
  return { h, s, l, a };
}

export function hslToRgb({ h, s, l, a }: HSLA): RGBA {
  if (s === 0) {
    const v = Math.round(l * 255);
    return { r: v, g: v, b: v, a };
  }
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  const [r, g, b] = (
    h < 60
      ? [c, x, 0]
      : h < 120
        ? [x, c, 0]
        : h < 180
          ? [0, c, x]
          : h < 240
            ? [0, x, c]
            : h < 300
              ? [x, 0, c]
              : [c, 0, x]
  ).map((n) => Math.round((n + m) * 255));
  return { r, g, b, a };
}

// WCAG 2 relative luminance, ignoring alpha.
export function relativeLuminance({ r, g, b }: RGBA): number {
  const ch = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b);
}

export function contrastRatio(a: RGBA, b: RGBA): number {
  const l1 = relativeLuminance(a);
  const l2 = relativeLuminance(b);
  return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
}

// `top` alpha-composited over an opaque `bottom`.
export function composite(top: RGBA, bottom: RGBA): RGBA {
  const a = top.a;
  return {
    r: top.r * a + bottom.r * (1 - a),
    g: top.g * a + bottom.g * (1 - a),
    b: top.b * a + bottom.b * (1 - a),
    a: 1,
  };
}

// Below this luminance a colour counts as dark: #212121 is 0.016, #777 is 0.18, #888 is 0.25.
export const DARK_LUMINANCE = 0.2;

export function isDarkColor(color: RGBA): boolean {
  return relativeLuminance(color) < DARK_LUMINANCE;
}

const MAX_BG_LIGHTNESS = 0.4;
const MIN_FG_LIGHTNESS = 0.55;

// Light backgrounds become dark, ending at the theme background for white; dark backgrounds
// stay dark. Greys and pale blues take the theme background's hue so a white page matches it.
export function darkBackground({ h, s, l, a }: HSLA, pole: HSLA): HSLA {
  const isBlue = h > 200 && h < 280;
  const isNeutral = s < 0.12 || (l > 0.8 && isBlue);
  if (l < 0.5) {
    const lx = scale(l, 0, 0.5, 0, MAX_BG_LIGHTNESS);
    return isNeutral ? { h: pole.h, s: pole.s, l: lx, a } : { h, s, l: lx, a };
  }

  let lx = scale(l, 0.5, 1, MAX_BG_LIGHTNESS, pole.l);
  if (isNeutral) return { h: pole.h, s: pole.s, l: lx, a };

  let hx = h;
  if (h > 60 && h < 180) {
    hx = h > 120 ? scale(h, 120, 180, 135, 180) : scale(h, 60, 120, 60, 105);
  }
  // Dark yellows read as muddy olive; push them darker.
  if (hx > 40 && hx < 80) lx *= 0.75;
  return { h: hx, s, l: lx, a };
}

function blueForegroundHue(h: number) {
  return scale(h, 205, 245, 205, 220);
}

// Every text colour ends up at or above MIN_FG_LIGHTNESS; black lands on the theme text colour
// and white stays near it, so text keeps its relative emphasis.
export function lightForeground({ h, s, l, a }: HSLA, pole: HSLA): HSLA {
  const isNeutral = l < 0.2 || s < 0.24;
  const isBlue = !isNeutral && h > 205 && h < 245;
  if (l > 0.5) {
    const lx = scale(l, 0.5, 1, MIN_FG_LIGHTNESS, pole.l);
    if (isNeutral) return { h: pole.h, s: pole.s, l: lx, a };
    return { h: isBlue ? blueForegroundHue(h) : h, s, l: lx, a };
  }
  if (isNeutral) {
    return { h: pole.h, s: pole.s, l: scale(l, 0, 0.5, pole.l, MIN_FG_LIGHTNESS), a };
  }
  if (isBlue) {
    const lx = scale(l, 0, 0.5, pole.l, Math.min(1, MIN_FG_LIGHTNESS + 0.05));
    return { h: blueForegroundHue(h), s, l: lx, a };
  }
  return { h, s, l: scale(l, 0, 0.5, pole.l, MIN_FG_LIGHTNESS), a };
}

export function darkBorder({ h, s, l, a }: HSLA, fgPole: HSLA, bgPole: HSLA): HSLA {
  const isNeutral = l < 0.2 || s < 0.24;
  let hx = h;
  let sx = s;
  if (isNeutral) {
    const pole = l < 0.5 ? fgPole : bgPole;
    hx = pole.h;
    sx = pole.s;
  }
  return { h: hx, s: sx, l: scale(l, 0, 1, 0.5, 0.2), a };
}

export type ColorRole = 'background' | 'foreground' | 'border';

export interface Poles {
  background: HSLA;
  foreground: HSLA;
}

export function polesFor(background: RGBA, text: RGBA): Poles {
  return { background: rgbToHsl(background), foreground: rgbToHsl(text) };
}

export function mapColor(color: RGBA, role: ColorRole, poles: Poles): RGBA {
  const hsl = rgbToHsl(color);
  const mapped =
    role === 'background'
      ? darkBackground(hsl, poles.background)
      : role === 'foreground'
        ? lightForeground(hsl, poles.foreground)
        : darkBorder(hsl, poles.foreground, poles.background);
  return hslToRgb(mapped);
}

// Maps computed colour strings, caching per role. Returns null when the value can't be parsed
// or is fully transparent, meaning "leave as is".
export class ColorMapper {
  private cache = new Map<string, string | null>();

  constructor(private poles: Poles) {}

  map(value: string, role: ColorRole): string | null {
    const key = `${role}|${value}`;
    const hit = this.cache.get(key);
    if (hit !== undefined) return hit;
    const parsed = parseColor(value);
    const result = parsed && parsed.a > 0 ? formatColor(mapColor(parsed, role, this.poles)) : null;
    this.cache.set(key, result);
    return result;
  }
}
