import fs from 'fs';
import path from 'path';
import { applyColorScheme, darkenDocument, formatColor, isDarkColor } from '@tern/email-dark';
import { darkPalette, themeBackgroundAt } from '../lib/email-render-mode';
import { compatTokensSource } from '../../../src/plugin-host/theme-manager';

// less comes with less-cache, which compiles the app's stylesheets at runtime.
// eslint-disable-next-line import/no-extraneous-dependencies
const less = require('less');

// Each bundled theme's stylesheets, compiled as ThemeManager would, around a copy of the message
// list's markup, so the colour behind the message iframe is the one the theme really paints.

const resourcePath = AppEnv.getLoadSettings().resourcePath;
const SYSTEM_ACCENT = ':root { --system-accent: #d14a8b; }';

const EMAIL = `
<table id="card" width="100%" bgcolor="#ffffff"><tr><td>
  <p id="plain">Thanks for your order. It ships tomorrow.</p>
  <p id="black" style="color: #000000">Order number 1042.</p>
</td></tr></table>`;

function compile(file: string, paths: string[], source = fs.readFileSync(file, 'utf8')) {
  let css = '';
  less.render(
    source,
    { filename: file, paths, syncImport: true },
    (err: Error, result: { css: string }) => {
      if (err) throw err;
      css = result.css;
    }
  );
  return css;
}

function themeDirs() {
  const dir = path.join(resourcePath, 'themes');
  return fs.readdirSync(dir).filter((name) => {
    const pkgPath = path.join(dir, name, 'package.json');
    if (!fs.existsSync(pkgPath) || !fs.existsSync(path.join(dir, name, 'styles'))) return false;
    return JSON.parse(fs.readFileSync(pkgPath, 'utf8')).theme === 'ui';
  });
}

function themeCss(theme: string) {
  const stylesDir = path.join(resourcePath, 'themes', theme, 'styles');
  const compat = AppEnv.themes.usesCompatTokens({ getStylesheetsPath: () => stylesDir });
  const paths = [
    stylesDir,
    ...(compat ? [path.join(resourcePath, 'static', 'style', 'compat')] : []),
    path.join(resourcePath, 'static', 'style'),
    path.join(resourcePath, 'static', 'style', 'base'),
  ];
  const tokensFile = path.join(stylesDir, 'tokens.css');
  const tokens = [
    fs.readFileSync(path.join(resourcePath, 'static', 'style', 'tokens', 'defaults.css'), 'utf8'),
    fs.existsSync(tokensFile) ? fs.readFileSync(tokensFile, 'utf8') : '',
    compat
      ? compile(
          path.join(resourcePath, 'static', 'style', 'compat', 'theme.less'),
          paths,
          compatTokensSource(resourcePath, stylesDir)
        )
      : '',
  ].join('\n');
  const app = [
    path.join(resourcePath, 'static', 'style', 'index.less'),
    path.join(resourcePath, 'features', 'message-list', 'styles', 'message-list.less'),
  ];
  const own = fs
    .readdirSync(stylesDir)
    .filter((f) => f.endsWith('.less'))
    .map((f) => path.join(stylesDir, f));
  return {
    tokens,
    app: [...app, ...own].map((f) => compile(f, paths)).join('\n'),
    emailFrame: compile(path.join(resourcePath, 'static', 'style', 'email-frame.less'), paths),
  };
}

describe('email darkening under the bundled themes', function () {
  let host: HTMLIFrameElement;

  afterEach(() => {
    if (host) host.remove();
  });

  const render = (theme: string, accent: boolean) => {
    const css = themeCss(theme);
    host = document.createElement('iframe');
    host.style.width = '1000px';
    host.style.height = '800px';
    document.body.appendChild(host);
    const hostDoc = host.contentDocument;
    hostDoc.open();
    hostDoc.write(
      `<!DOCTYPE html><style>${css.tokens}</style><style>${accent ? SYSTEM_ACCENT : ''}</style>` +
        `<style>${css.app}</style>` +
        `<style>${css.emailFrame}</style>` +
        `<body class="platform-darwin theme-ui-light theme-${theme}"><div id="message-list">` +
        `<div class="messages-wrap"><div class="scroll-region-content-inner">` +
        `<div class="message-item-wrap"><div class="message-item-white-wrap">` +
        `<div class="message-item-area"><span><div class="message-iframe-container">` +
        `<iframe id="email" sandbox="allow-same-origin"></iframe>` +
        `</div></span></div></div></div></div></div></div></body>`
    );
    hostDoc.close();
    const hostWin = hostDoc.defaultView;

    // What EmailFrameStylesStore.themeColors reads in the app window.
    const scope = hostDoc.createElement('span');
    scope.className = 'ignore-in-parent-frame';
    scope.innerHTML = '<div id="inbox-html-wrapper"><a>link</a></div>';
    hostDoc.body.appendChild(scope);
    const text = hostWin.getComputedStyle(scope.firstElementChild).color;
    const link = hostWin.getComputedStyle(scope.querySelector('a')).color;
    scope.remove();

    const frame = hostDoc.getElementById('email') as HTMLIFrameElement;
    const card = hostWin.getComputedStyle(hostDoc.querySelector('.message-item-white-wrap'));
    const doc = frame.contentDocument;
    doc.open();
    doc.write(
      `<!DOCTYPE html><style>${css.tokens}${accent ? SYSTEM_ACCENT : ''}` +
        `${css.emailFrame.replace(/.ignore-in-parent-frame/g, '')}</style>` +
        `<div id="inbox-html-wrapper">${EMAIL}</div>`
    );
    doc.close();
    return { frame, doc, text, link, cardColor: card.backgroundColor };
  };

  const themes = themeDirs();
  const darkThemes: string[] = [];

  for (const theme of themes) {
    for (const accent of [false, true]) {
      it(`${theme}${accent ? ' with the system accent' : ''}`, () => {
        const { frame, doc, text, link, cardColor } = render(theme, accent);
        const behind = themeBackgroundAt(frame);
        expect(formatColor(behind)).toBe(cardColor);
        if (!isDarkColor(behind)) return;
        if (!accent) darkThemes.push(theme);

        const palette = darkPalette(behind, text, link);
        expect(palette.text).toBe(text);
        expect(palette.link).toBe(link);

        applyColorScheme(frame, 'dark');
        const root = doc.getElementById('inbox-html-wrapper');
        expect(darkenDocument(doc, palette, { root }).status).toBe('darkened');
        const view = doc.defaultView;
        expect(view.getComputedStyle(doc.getElementById('card')).backgroundColor).toBe(cardColor);
        expect(view.getComputedStyle(root).backgroundColor).toBe(cardColor);
        expect(view.getComputedStyle(doc.getElementById('plain')).color).toBe(text);
        expect(view.getComputedStyle(doc.getElementById('black')).color).toBe(text);
      });
    }
  }

  it('covers at least the Dark theme', () => {
    expect(darkThemes).toContain('ui-dark');
  });
});
