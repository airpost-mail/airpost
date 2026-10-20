import {
  THEME_TOKENS_SOURCE_PATH,
  TOKEN_DEFAULTS_SOURCE_PATH,
} from '../src/plugin-host/theme-manager';

// Theme tokens as ThemeManager loads them (docs/desktop/theme-tokens.md), with the fixture themes in
// spec/fixtures/packages: theme-nord-variables only sets LESS variables, theme-with-tokens ships
// styles/tokens.css and declares its scheme.

const token = (name: string) =>
  window.getComputedStyle(document.documentElement).getPropertyValue(name).trim();

const sheet = (sourcePath: string) =>
  document.querySelector(`managed-styles [source-path="${sourcePath}"]`) as HTMLStyleElement;

function activate(themeName: string) {
  AppEnv.config.set('core.theme', themeName);
  AppEnv.themes.updateThemePackageAndRecomputeLESS();
}

describe('theme tokens', function () {
  let probe: HTMLElement;

  beforeEach(() => {
    probe = document.createElement('div');
    document.body.appendChild(probe);
  });

  afterEach(() => {
    probe.remove();
    activate('ui-light');
  });

  const colorOf = (value: string) => {
    probe.style.color = value;
    return window.getComputedStyle(probe).color;
  };

  it('loads the defaults at core priority and nothing at theme priority for Light', () => {
    activate('ui-light');
    const defaults = sheet(TOKEN_DEFAULTS_SOURCE_PATH);
    expect(defaults).toBeTruthy();
    expect((defaults as any).priority).toBe(-1);
    expect(sheet(THEME_TOKENS_SOURCE_PATH)).toBe(null);
    expect(colorOf('var(--background)')).toBe('rgb(255, 255, 255)');
    expect(colorOf('var(--foreground)')).toBe('rgb(35, 31, 32)');
    expect(document.body.classList.contains('theme-scheme-light')).toBe(true);
    expect(document.body.classList.contains('theme-sidebar-scheme-light')).toBe(true);
  });

  describe('a theme with styles/tokens.css', () => {
    beforeEach(() => activate('theme-with-tokens'));

    it('loads its tokens after the defaults, so they win', () => {
      const all = Array.from(document.querySelectorAll('managed-styles > style'));
      const theme = sheet(THEME_TOKENS_SOURCE_PATH);
      expect(theme.textContent).toContain('--background: #102030');
      expect((theme as any).priority).toBe(1);
      expect(all.indexOf(sheet(TOKEN_DEFAULTS_SOURCE_PATH))).toBeLessThan(all.indexOf(theme));
      expect(colorOf('var(--background)')).toBe('rgb(16, 32, 48)');
    });

    it("keeps defaults for what it doesn't set, and derived tokens follow what it does", () => {
      expect(colorOf('var(--foreground)')).toBe('rgb(35, 31, 32)');
      expect(colorOf('var(--sidebar-background)')).toBe('rgb(64, 80, 96)');
      expect(colorOf('var(--rail-selected)')).toBe('rgb(16, 32, 48)');
    });

    it('loads tokens.css once, not again as one of its stylesheets', () => {
      const copies = Array.from(document.querySelectorAll('managed-styles > style')).filter((el) =>
        el.textContent.includes('#102030')
      );
      expect(copies.length).toBe(1);
    });

    it('compiles its LESS against the token variables', () => {
      const own = Array.from(document.querySelectorAll('managed-styles > style')).find((el) =>
        el.textContent.includes('.theme-with-tokens-probe')
      );
      expect(own.textContent).toContain('color: var(--foreground)');
    });

    it('takes its scheme from package.json, and the sidebar follows it', () => {
      expect(document.body.classList.contains('theme-scheme-dark')).toBe(true);
      expect(document.body.classList.contains('theme-sidebar-scheme-dark')).toBe(true);
      expect(document.body.classList.contains('theme-scheme-light')).toBe(false);
    });
  });

  describe('a theme with only LESS variables', () => {
    let css: string;
    beforeEach(() => {
      activate('theme-nord-variables');
      css = sheet(THEME_TOKENS_SOURCE_PATH).textContent;
    });

    it('gets tokens derived from its variables', () => {
      expect(colorOf('var(--background)')).toBe('rgb(46, 52, 64)');
      expect(colorOf('var(--foreground)')).toBe('rgb(236, 239, 244)');
      expect(colorOf('var(--heading-foreground)')).toBe('rgb(216, 222, 233)');
      expect(colorOf('var(--success)')).toBe('rgb(163, 190, 140)');
      expect(colorOf('var(--danger)')).toBe('rgb(191, 97, 106)');
      expect(colorOf('var(--sidebar-background)')).toBe('rgb(59, 66, 82)');
      expect(colorOf('var(--thread-list-background)')).toBe('rgb(55, 62, 76)');
      expect(css).toContain('--primary: var(--system-accent, #88c0d0)');
    });

    it('derives what it leaves unset from the literal defaults and its own values', () => {
      // @text-color-subtle is fadeout(@text-color, 15%) in compat/base/ui-variables.less.
      expect(colorOf('var(--subtle-foreground)')).toBe('rgba(236, 239, 244, 0.85)');
      expect(colorOf('var(--warning)')).toBe('rgb(240, 173, 78)');
      expect(colorOf('var(--link)')).toBe('rgb(31, 112, 236)');
      // No LESS source: defaults.css derives it from the compat --foreground.
      expect(colorOf('var(--accent)')).toBe(colorOf('color-mix(in srgb, #eceff4 5%, transparent)'));
    });

    it('never sets a token to itself', () => {
      for (const [, name, value] of css.matchAll(/(--[\w-]+):\s*([^;]+);/g)) {
        expect(value.includes(`var(${name})`)).toBe(false);
      }
    });

    it('compiles LESS against literal values, so colour functions on unset variables work', () => {
      expect(colorOf('var(--control)')).toBe('rgb(78, 85, 99)');
      const index = Array.from(document.querySelectorAll('managed-styles > style')).find((el) =>
        ((el as any).sourcePath || '').endsWith('index.less')
      );
      expect(index.textContent).toContain('background-color: #2e3440');
      expect(index.textContent).not.toContain('var(--background)');
    });

    it('derives its schemes from its backgrounds', () => {
      expect(document.body.classList.contains('theme-scheme-dark')).toBe(true);
      expect(document.body.classList.contains('theme-sidebar-scheme-dark')).toBe(true);
    });

    it('drops its tokens when another theme takes over', () => {
      activate('ui-light');
      expect(sheet(THEME_TOKENS_SOURCE_PATH)).toBe(null);
      expect(colorOf('var(--background)')).toBe('rgb(255, 255, 255)');
      expect(document.body.classList.contains('theme-scheme-light')).toBe(true);
    });
  });
});
