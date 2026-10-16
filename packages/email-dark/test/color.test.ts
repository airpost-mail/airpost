// SPDX-License-Identifier: GPL-3.0-only

import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  ColorMapper,
  RGBA,
  contrastRatio,
  formatColor,
  hslToRgb,
  mapColor,
  parseColor,
  polesFor,
  relativeLuminance,
  rgbToHsl,
} from '../src/color';

const THEME_BG = parseColor('#212121') as RGBA;
const THEME_TEXT = parseColor('#eeeeee') as RGBA;
const poles = polesFor(THEME_BG, THEME_TEXT);

const grey = (v: number): RGBA => ({ r: v, g: v, b: v, a: 1 });
const close = (a: RGBA, b: RGBA, tolerance = 2) =>
  Math.abs(a.r - b.r) <= tolerance &&
  Math.abs(a.g - b.g) <= tolerance &&
  Math.abs(a.b - b.b) <= tolerance;

test('parses computed and authored colour forms', () => {
  assert.deepEqual(parseColor('rgb(1, 2, 3)'), { r: 1, g: 2, b: 3, a: 1 });
  assert.deepEqual(parseColor('rgba(1, 2, 3, 0.5)'), { r: 1, g: 2, b: 3, a: 0.5 });
  assert.deepEqual(parseColor('rgb(1 2 3 / 50%)'), { r: 1, g: 2, b: 3, a: 0.5 });
  assert.deepEqual(parseColor('color(srgb 1 0 0.5)'), { r: 255, g: 0, b: 128, a: 1 });
  assert.deepEqual(parseColor('#fff'), { r: 255, g: 255, b: 255, a: 1 });
  assert.deepEqual(parseColor('#336699'), { r: 51, g: 102, b: 153, a: 1 });
  assert.deepEqual(parseColor('#33669980'), { r: 51, g: 102, b: 153, a: 128 / 255 });
  assert.deepEqual(parseColor('transparent'), { r: 0, g: 0, b: 0, a: 0 });
  assert.equal(parseColor('currentcolor'), null);
  assert.equal(parseColor('url(#gradient)'), null);
  assert.equal(parseColor(''), null);
});

test('formats like Chromium computed style', () => {
  assert.equal(formatColor({ r: 10, g: 20, b: 30, a: 1 }), 'rgb(10, 20, 30)');
  assert.equal(formatColor({ r: 10, g: 20, b: 30, a: 0.25 }), 'rgba(10, 20, 30, 0.25)');
});

test('HSL conversion round-trips', () => {
  for (const hex of ['#000000', '#ffffff', '#ff0000', '#1a73e8', '#fff3e0', '#7f7f7f']) {
    const rgb = parseColor(hex) as RGBA;
    assert.ok(close(hslToRgb(rgbToHsl(rgb)), rgb, 1), hex);
  }
});

test('a white background becomes the theme background', () => {
  assert.ok(close(mapColor(grey(255), 'background', poles), THEME_BG));
});

test('black text becomes the theme text colour', () => {
  assert.ok(close(mapColor(grey(0), 'foreground', poles), THEME_TEXT));
});

test('light backgrounds become dark and keep their order', () => {
  let previous = -1;
  for (let v = 128; v <= 255; v += 8) {
    const mapped = mapColor(grey(v), 'background', poles);
    const l = relativeLuminance(mapped);
    assert.ok(l < 0.2, `grey ${v} -> ${formatColor(mapped)}`);
    if (previous >= 0) assert.ok(l <= previous + 1e-9, `grey ${v} should not be lighter`);
    previous = l;
  }
});

test('dark text becomes light and keeps its order', () => {
  let previous = 2;
  for (let v = 0; v <= 128; v += 8) {
    const l = relativeLuminance(mapColor(grey(v), 'foreground', poles));
    assert.ok(l <= previous + 1e-9, `grey ${v} should not be lighter than a darker grey`);
    previous = l;
  }
});

test('text on a white page stays legible after mapping', () => {
  const page = mapColor(grey(255), 'background', poles);
  for (const hex of ['#000000', '#333333', '#666666', '#999999', '#1a73e8', '#d93025', '#188038']) {
    const text = mapColor(parseColor(hex) as RGBA, 'foreground', poles);
    assert.ok(contrastRatio(text, page) >= 3, `${hex} -> ${formatColor(text)}`);
  }
});

test('white text on a dark button stays readable', () => {
  const button = mapColor(parseColor('#1a73e8') as RGBA, 'background', poles);
  const label = mapColor(grey(255), 'foreground', poles);
  assert.ok(contrastRatio(label, button) >= 3);
});

test('coloured backgrounds keep their hue and saturation', () => {
  const orange = parseColor('#ffe0b2') as RGBA;
  const before = rgbToHsl(orange);
  const after = rgbToHsl(mapColor(orange, 'background', poles));
  assert.ok(Math.abs(before.h - after.h) < 3, `hue ${before.h} -> ${after.h}`);
  assert.ok(after.s > before.s * 0.8, `saturation ${before.s} -> ${after.s}`);
  assert.ok(after.l < 0.4);
});

test('greys take the theme hue', () => {
  const warmBg = parseColor('#2a2420') as RGBA;
  const warm = polesFor(warmBg, THEME_TEXT);
  assert.ok(close(mapColor(grey(255), 'background', warm), warmBg));
  const mapped = rgbToHsl(mapColor(grey(240), 'background', warm));
  assert.ok(Math.abs(mapped.h - warm.background.h) < 6, `hue ${mapped.h}`);
});

test('borders become dim lines', () => {
  const light = relativeLuminance(mapColor(parseColor('#dddddd') as RGBA, 'border', poles));
  assert.ok(light < 0.1);
});

test('alpha is preserved', () => {
  assert.equal(mapColor({ r: 0, g: 0, b: 0, a: 0.3 }, 'background', poles).a, 0.3);
});

test('ColorMapper caches and skips what it cannot map', () => {
  const mapper = new ColorMapper(poles);
  const first = mapper.map('rgb(255, 255, 255)', 'background');
  assert.equal(first, formatColor(mapColor(grey(255), 'background', poles)));
  assert.equal(mapper.map('rgb(255, 255, 255)', 'background'), first);
  assert.notEqual(mapper.map('rgb(255, 255, 255)', 'foreground'), first);
  assert.equal(mapper.map('rgba(0, 0, 0, 0)', 'background'), null);
  assert.equal(mapper.map('none', 'foreground'), null);
});
