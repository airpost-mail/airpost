// SPDX-License-Identifier: GPL-3.0-only

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { RGBA, parseColor } from '../src/color';
import { darkShare, isAlreadyDark } from '../src/detect';

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
