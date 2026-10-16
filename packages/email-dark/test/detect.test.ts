// SPDX-License-Identifier: GPL-3.0-only

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { RGBA, parseColor } from '../src/color';
import { DECLARED_DARK_SHARE, darkShare, isAlreadyDark, textDarkShare } from '../src/detect';

const c = (hex: string) => parseColor(hex) as RGBA;

test('a light newsletter is not dark', () => {
  const regions = [
    { color: c('#ffffff'), area: 300000 },
    { color: c('#f4f4f4'), area: 200000 },
    { color: c('#202124'), area: 40000 }, // dark footer
  ];
  assert.equal(isAlreadyDark(regions), false);
});

test('an email whose wrapper table is dark is dark', () => {
  const regions = [
    { color: c('#ffffff'), area: 20000 }, // margin outside the table
    { color: c('#121212'), area: 400000 },
    { color: c('#ffffff'), area: 30000 }, // a light card
  ];
  assert.equal(isAlreadyDark(regions), true);
});

test('images and gradients do not vote', () => {
  const regions = [
    { color: c('#000000'), area: 10000 },
    { color: null, area: 900000 },
    { color: c('#ffffff'), area: 5000 },
  ];
  assert.ok(Math.abs(darkShare(regions) - 10000 / 15000) < 1e-9);
  assert.equal(isAlreadyDark(regions), true);
});

test('regions covered by their children count for nothing', () => {
  const regions = [
    { color: c('#000000'), area: -50 },
    { color: c('#ffffff'), area: 100 },
  ];
  assert.equal(darkShare(regions), 0);
});

test('no painted area is not dark', () => {
  assert.equal(isAlreadyDark([]), false);
});

const text = (fg: string, backdrop: string | null, weight = 100) => ({
  color: c(fg),
  backdrop: backdrop ? c(backdrop) : null,
  weight,
});

test('text decides when there is enough of it', () => {
  // A small dark table in a wide pane: little dark area, but all the text is on it.
  const regions = [
    { color: c('#0f1115'), area: 1000 },
    { color: null, area: 100000 },
  ];
  assert.equal(isAlreadyDark(regions, [text('#e6e6e6', '#0f1115', 200)]), true);
  // A light newsletter with a big dark hero but most text on white.
  const light = [text('#ffffff', '#000000', 60), text('#333333', '#ffffff', 300)];
  assert.equal(isAlreadyDark([{ color: c('#000000'), area: 900000 }], light), false);
});

test('light text on unknown backdrop counts as dark, dark text as light', () => {
  assert.deepEqual(textDarkShare([text('#f1f1f1', null)]), { share: 1, weight: 100 });
  assert.deepEqual(textDarkShare([text('#202124', null)]), { share: 0, weight: 100 });
});

test('little text falls back to painted area', () => {
  const regions = [{ color: c('#111111'), area: 100 }];
  assert.equal(isAlreadyDark(regions, [text('#000000', '#ffffff', 10)]), true);
});

test('a lower threshold applies to emails that declare dark support', () => {
  const texts = [text('#eeeeee', '#121212', 30), text('#222222', '#ffffff', 70)];
  assert.equal(isAlreadyDark([], texts), false);
  assert.equal(isAlreadyDark([], texts, DECLARED_DARK_SHARE), true);
});
