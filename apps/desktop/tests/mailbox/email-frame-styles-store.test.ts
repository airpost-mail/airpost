import { EmailFrameStylesStore } from '../../src/features/message-list/email-frame-styles-store';

describe('EmailFrameStylesStore', () => {
  let store: EmailFrameStylesStore;
  let coreSheet: HTMLStyleElement;
  let themeSheet: HTMLStyleElement;

  beforeEach(() => {
    coreSheet = document.createElement('style');
    coreSheet.setAttribute('source-path', '/static/styles/email-frame.css');
    coreSheet.innerText = '.ignore-in-parent-frame body { font-family: test; }';
    document.head.appendChild(coreSheet);

    themeSheet = document.createElement('style');
    themeSheet.setAttribute('source-path', '/themes/example/email-frame.less');
    themeSheet.innerText = '.ignore-in-parent-frame body { font-size: 18px; color: pink; }';
    document.head.appendChild(themeSheet);

    store = new EmailFrameStylesStore();
  });

  afterEach(() => {
    store._unlistenToStyles();
    coreSheet.remove();
    themeSheet.remove();
  });

  it('passes theme typography and colors through to the email frame', () => {
    store._findStyles();
    const { themeStyles } = store.styles();

    expect(themeStyles).toContain('font-family: test');
    expect(themeStyles).toContain('font-size: 18px; color: pink;');
    expect(themeStyles).not.toContain('ignore-in-parent-frame');
  });

  it('sends the theme tokens with the email frame styles, in document order', () => {
    const tokens = document.createElement('style');
    tokens.setAttribute('source-path', 'tokens:theme');
    tokens.innerText = ':root { --foreground: rgb(7, 8, 9); }';
    document.head.insertBefore(tokens, coreSheet);
    store._findStyles();
    tokens.remove();

    const { themeStyles } = store.styles();
    expect(themeStyles).toContain('--foreground: rgb(7, 8, 9)');
    expect(themeStyles.indexOf('--foreground')).toBeLessThan(themeStyles.indexOf('font-family'));
  });

  it('sends the core variables and an email-frame stylesheet in CSS', () => {
    const variables = document.createElement('style');
    variables.setAttribute('source-path', AppEnv.themes.coreVariablesPath());
    variables.innerText = ':root { --spacing-test: 3px; }';
    document.head.insertBefore(variables, coreSheet);
    const cssSheet = document.createElement('style');
    cssSheet.setAttribute('source-path', '/themes/other/styles/email-frame.css');
    cssSheet.innerText = '.ignore-in-parent-frame body { letter-spacing: 1px; }';
    document.head.appendChild(cssSheet);
    const unrelated = document.createElement('style');
    unrelated.setAttribute('source-path', '/themes/other/styles/variables.css');
    unrelated.innerText = ':root { --unrelated: 1px; }';
    document.head.appendChild(unrelated);
    store._findStyles();
    variables.remove();
    cssSheet.remove();
    unrelated.remove();

    const { themeStyles } = store.styles();
    expect(themeStyles).toContain('--spacing-test: 3px');
    expect(themeStyles).toContain('letter-spacing: 1px');
    expect(themeStyles).not.toContain('--unrelated');
    expect(themeStyles.indexOf('--spacing-test')).toBeLessThan(
      themeStyles.indexOf('font-family: test')
    );
  });

  it('reads the theme text and link colors the email frame uses', () => {
    themeSheet.innerText +=
      '\n.ignore-in-parent-frame #inbox-html-wrapper { color: rgb(1, 2, 3); }' +
      '\n.ignore-in-parent-frame a { color: rgb(4, 5, 6); }';
    store._findStyles();

    expect(store.themeColors()).toEqual({ text: 'rgb(1, 2, 3)', link: 'rgb(4, 5, 6)' });
  });
});
