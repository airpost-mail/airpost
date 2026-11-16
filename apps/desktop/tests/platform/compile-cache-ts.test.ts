import fs from 'fs';
import os from 'os';
import path from 'path';

// Specs load through compile-cache-ts (static/index.js), so requiring a .ts file here goes
// through the same cache the app uses.
describe('compile-cache-ts', () => {
  let root: string;

  beforeEach(() => {
    root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'airpost-compile-cache-')));
  });

  afterEach(() => {
    for (const key of Object.keys(require.cache)) {
      if (key.startsWith(root)) delete require.cache[key];
    }
    fs.rmSync(root, { recursive: true, force: true });
  });

  it('keeps the path of a file whose content another path already compiled', () => {
    // Same bytes at two paths, as after a rename: the second must not reuse the first's
    // sourceURL, which DevTools, console sources and V8's own stack format show.
    const source = [
      'export const probe = (): string => {',
      '  const prepare = Error.prepareStackTrace;',
      '  Error.prepareStackTrace = (_error, frames) => frames[0].getScriptNameOrSourceURL();',
      '  try {',
      '    return new Error().stack;',
      '  } finally {',
      '    Error.prepareStackTrace = prepare;',
      '  }',
      '};',
      '',
    ].join('\n');
    const a = path.join(root, 'old-dir', 'probe.ts');
    const b = path.join(root, 'new-dir', 'probe.ts');
    for (const file of [a, b]) {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, source);
    }

    expect(require(a).probe()).toBe(a);
    expect(require(b).probe()).toBe(b);
  });
});
