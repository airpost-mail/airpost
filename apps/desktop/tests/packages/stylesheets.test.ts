import fs from 'fs';
import os from 'os';
import path from 'path';
import { CORE_STYLESHEETS } from '../../src/packages/theme-manager';
import { stylesheetsIn } from '../../src/packages/resources';

// How ThemeManager loads stylesheets: the app's own are plain CSS, LESS is compiled for
// third-party packages only, against the LESS surface in static/styles/compat/less.

const resourcePath = () => AppEnv.getLoadSettings().resourcePath;
const managed = () => Array.from(document.querySelectorAll('managed-styles > style')) as any[];

describe('core stylesheets', function () {
  it('load in CORE_STYLESHEETS order at core priority, before email-frame and the kit', () => {
    const paths = AppEnv.themes.coreStylesheetPaths();
    expect(paths.length).toBe(CORE_STYLESHEETS.length);
    expect(path.basename(paths[0])).toBe('font-awesome.min.css');
    expect(paths[1]).toBe(AppEnv.themes.coreVariablesPath());

    const sources = managed().map((el) => el.sourcePath);
    const positions = paths.map((p) => sources.indexOf(p));
    expect(positions.every((i) => i >= 0)).toBe(true);
    expect([...positions].sort((a, b) => a - b)).toEqual(positions);
    for (const i of positions) expect(managed()[i].priority).toBe(-1);

    const emailFrame = sources.findIndex((s) => /email-frame\.css$/.test(s || ''));
    const kit = sources.indexOf(path.join(resourcePath(), 'static', 'styles', 'kit', 'base.css'));
    expect(positions[positions.length - 1]).toBeLessThan(emailFrame);
    expect(emailFrame).toBeLessThan(kit);
  });

  it('read CSS as it is, so the core variables are custom properties on :root', () => {
    const file = AppEnv.themes.coreVariablesPath();
    const el = managed().find((s) => s.sourcePath === file);
    expect(el.textContent).toBe(fs.readFileSync(file, 'utf8'));
    const value = (name: string) =>
      window.getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    expect(value('--spacing-standard')).toBe('14px');
    expect(value('--font-size-small')).toBe('13.02px');
  });

  it("run the app's CSS through rtlcss for an RTL locale, and nothing else", () => {
    const bundled = path.join(resourcePath(), 'plugins', 'example', 'styles', 'example.css');
    const kit = path.join(resourcePath(), 'static', 'styles', 'kit', 'example.css');
    const thirdParty = path.join(os.tmpdir(), 'tern-third-party', 'styles', 'example.css');
    const css = '.probe { margin-left: 3px; }';
    const readFileSync = fs.readFileSync;
    spyOn(fs, 'readFileSync').andCallFake((p: any, ...rest: any[]) =>
      [bundled, kit, thirdParty].includes(p) ? css : (readFileSync as any).call(fs, p, ...rest)
    );
    const rtl = AppEnv.themes.rtl;
    AppEnv.themes.rtl = true;
    try {
      expect(AppEnv.themes.cssContentsOfStylesheet(bundled)).toContain('margin-right: 3px');
      expect(AppEnv.themes.cssContentsOfStylesheet(kit)).toBe(css);
      expect(AppEnv.themes.cssContentsOfStylesheet(thirdParty)).toBe(css);
    } finally {
      AppEnv.themes.rtl = rtl;
    }
    expect(AppEnv.themes.cssContentsOfStylesheet(bundled)).toBe(css);
  });

  it('know which stylesheets are the app’s own', () => {
    const r = resourcePath();
    const own = (...p: string[]) => AppEnv.themes.isAppStylesheet(path.join(r, ...p));
    expect(own('static', 'styles', 'workspace.css')).toBe(true);
    expect(own('static', 'styles', 'components', 'menu.css')).toBe(true);
    expect(own('src', 'features', 'composer', 'styles', 'composer.css')).toBe(true);
    expect(own('plugins', 'ai', 'styles', 'ai.css')).toBe(true);
    expect(own('themes', 'ui-dark', 'styles', 'theme.css')).toBe(true);
    expect(own('static', 'styles', 'kit', 'button.css')).toBe(false);
    expect(own('static', 'styles', 'tokens', 'defaults.css')).toBe(false);
    expect(own('static', 'styles', 'compat', 'tokens.less')).toBe(false);
    expect(own('static', 'font-awesome.min.css')).toBe(false);
    expect(AppEnv.themes.isAppStylesheet(path.join(os.tmpdir(), 'pkg', 'styles', 'a.css'))).toBe(
      false
    );
  });
});

describe('a third-party package in LESS', function () {
  let dir: string;

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-less-package-'));
    fs.mkdirSync(path.join(dir, 'styles'));
  });

  afterEach(() => {
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('still imports the LESS surface and uses its variables and mixins', () => {
    const file = path.join(dir, 'styles', 'plugin.less');
    fs.writeFileSync(
      file,
      [
        `@import 'ui-variables';`,
        `@import 'ui-mixins';`,
        `@import 'base/ui-variables';`,
        `@import 'mixins/label-colors';`,
        `.probe { font-size: @font-size-small; color: @text-color; padding: @padding-base-vertical; }`,
        `.probe-bg { .bg-variant(#ffffff); }`,
        `.probe-chip { .label-chip-bounds(#000000); }`,
        `.probe-btn { .windows-btn-bg; }`,
      ].join('\n')
    );
    const css = AppEnv.themes.cssContentsOfStylesheet(file);
    expect(css).toContain('font-size: 13.02px');
    expect(css).toContain('color: var(--foreground)');
    expect(css).toContain('padding: 5px');
    expect(css).toContain('background-color: #ffffff');
    expect(css).toContain('background-color: color-mix(in srgb, #ffffff, black 10%)');
    expect(css).toContain('--label-chip-lmin: 0.28');
    expect(css).toContain('.item-count-box');
    expect(css).toContain('transition: background 300ms, box-shadow 300ms');
  });
});

describe('stylesheetsIn', function () {
  let dir: string;

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-styles-'));
  });

  afterEach(() => {
    fs.rmSync(dir, { recursive: true, force: true });
  });

  const names = (paths: string[]) => paths.map((p) => path.basename(p));

  it('loads every stylesheet by name, the extension counting as .css, and skips a theme’s tokens', () => {
    for (const f of ['b.less', 'a.css', 'a-x.less', 'tokens.css', 'notes.txt']) {
      fs.writeFileSync(path.join(dir, f), '');
    }
    expect(names(stylesheetsIn(dir, { isTheme: false }))).toEqual([
      'a-x.less',
      'a.css',
      'b.less',
      'tokens.css',
    ]);
    expect(names(stylesheetsIn(dir, { isTheme: true }))).toEqual(['a-x.less', 'a.css', 'b.less']);
  });

  it('loads styles/index.* alone', () => {
    for (const f of ['index.css', 'a.css']) fs.writeFileSync(path.join(dir, f), '');
    expect(names(stylesheetsIn(dir, { isTheme: false }))).toEqual(['index.css']);
  });

  it('keeps both foo.less and foo.css', () => {
    for (const f of ['foo.less', 'foo.css']) fs.writeFileSync(path.join(dir, f), '');
    expect(names(stylesheetsIn(dir, { isTheme: false }))).toEqual(['foo.css', 'foo.less']);
  });
});
