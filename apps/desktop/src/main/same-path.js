// SPDX-License-Identifier: GPL-3.0-only
const fs = require('fs');
const path = require('path');

function realpath(p) {
  const resolved = path.resolve(p);
  try {
    return fs.realpathSync(resolved);
  } catch (err) {
    return resolved;
  }
}

/** Whether two paths name the same file once symlinks are resolved (a path that doesn't
 * exist compares as given, resolved against the working directory). main.js loads this before
 * the TypeScript compile cache. */
function isSamePath(a, b) {
  return realpath(a) === realpath(b);
}

module.exports = { isSamePath };
