import path from 'path';
import Package from '../../src/packages/package';
import {
  parseThemePreviewColors,
  themePreviewColors,
  themePreviewColorsCSS,
} from '../../src/packages/theme-preview';

// The colours the theme previews draw (Preferences > Appearance, the theme picker): a bundled
// theme's styles/theme-colors.css, a LESS-variables theme's variables, else Light's.

const { resourcePath } = AppEnv.getLoadSettings();
const theme = (dir: string) => new Package(path.join(resourcePath, dir));
const fixture = (name: string) =>
  theme(path.join('tests', 'support', 'fixtures', 'packages', name));

const LIGHT = {
  sidebar: '#f1f3f7',
  content: '#ffffff',
  accent: 'var(--system-accent, #1f70ec)',
  text: '#231f20',
  background: '#f4f6f9',
  active: 'var(--system-accent-dark, color-mix(in srgb, var(--system-accent, #1f70ec), black 10%))',
  toolbar: '#cfd4dc',
  panel: '#f1f3f7',
  'swatch-border': 'rgba(0, 0, 0, 0.15)',
};

describe('theme previews', function () {
  it("read a bundled theme's theme-colors.css", () => {
    expect(themePreviewColors(theme('themes/ui-light'))).toEqual(LIGHT);
    expect(themePreviewColors(theme('themes/ui-dark'))).toEqual({
      sidebar: '#0b0b0b',
      content: '#151515',
      accent: 'var(--system-accent, #4d8ff4)',
      text: '#e8e8e8',
      background: '#111111',
      active:
        'var(--system-accent-dark, color-mix(in srgb, var(--system-accent, #4d8ff4), black 28%))',
      toolbar: '#0b0b0b',
      panel: '#0b0b0b',
      'swatch-border': 'rgba(232, 232, 232, 0.35)',
    });
    expect(themePreviewColors(theme('themes/ui-darkside')).accent).toBe('#f18260');
  });

  it("compile a LESS-variables theme's variables over the literal base", () => {
    expect(themePreviewColors(fixture('theme-nord-variables'))).toEqual({
      sidebar: '#3b4252',
      content: '#2e3440',
      accent: 'var(--system-accent, #88c0d0)',
      text: '#eceff4',
      background: '#282d37',
      active:
        'var(--system-accent-dark, color-mix(in srgb, var(--system-accent, #88c0d0), black 10%))',
      toolbar: '#cfd4dc',
      panel: '#f1f3f7',
      'swatch-border': 'rgba(0, 0, 0, 0.15)',
      font: "'Albert Sans', 'Helvetica', sans-serif",
      'font-weight': 'var(--font-weight-semi-bold)',
      ring: '#1f70ec',
    });
  });

  it("use Light's for a theme with neither", () => {
    expect(themePreviewColors(fixture('theme-with-tokens'))).toEqual(LIGHT);
  });

  it('round-trip through the rule the picker iframe gets', () => {
    const css = themePreviewColorsCSS(LIGHT);
    expect(css.startsWith(':root {')).toBe(true);
    expect(parseThemePreviewColors(css)).toEqual(LIGHT);
  });
});
