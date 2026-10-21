// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import path from 'path';

// The quick-preview libraries are copied into vendor/, where npm audit doesn't see them.
describe('the vendored quick-preview libraries', function () {
  const appDir = AppEnv.getLoadSettings().resourcePath;
  const vendor = path.join(appDir, 'vendor');
  const renderer = fs.readFileSync(
    path.join(appDir, 'src', 'mail', 'quick-preview', 'renderer.html'),
    'utf8'
  );

  it('are the copies renderer.html loads', () => {
    const referenced = [...renderer.matchAll(/vendor\/([^'"`]+)/g)].map((m) => m[1]);
    expect(referenced.length).toBeGreaterThan(0);
    for (const rel of referenced) {
      expect(fs.existsSync(path.join(vendor, rel))).toBe(true);
    }
  });

  it('keep DOMPurify at the version the app depends on', () => {
    const vendored = fs.readFileSync(path.join(vendor, 'dompurify', 'purify.min.js'), 'utf8');
    const { version } = JSON.parse(
      fs.readFileSync(path.join(appDir, 'node_modules', 'dompurify', 'package.json'), 'utf8')
    );
    expect(vendored.slice(0, 200)).toContain(`DOMPurify ${version} `);
  });

  it('carry SheetJS 0.20.2 or later (CVE-2023-30533, CVE-2024-22363)', () => {
    const [major, minor, patch] = fs
      .readdirSync(vendor)
      .find((name) => name.startsWith('xlsx-'))
      .slice('xlsx-'.length)
      .split('.')
      .map(Number);
    expect(major * 1e6 + minor * 1e3 + patch).not.toBeLessThan(20002);
  });
});
