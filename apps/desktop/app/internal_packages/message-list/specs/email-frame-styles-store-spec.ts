import { EmailFrameStylesStore } from '../lib/email-frame-styles-store';

describe('EmailFrameStylesStore', () => {
  let store: EmailFrameStylesStore;
  let coreSheet: HTMLStyleElement;
  let themeSheet: HTMLStyleElement;

  beforeEach(() => {
    coreSheet = document.createElement('style');
    coreSheet.setAttribute('source-path', '/static/style/email-frame.less');
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

  it('reads the theme text and link colors the email frame uses', () => {
    themeSheet.innerText +=
      '\n.ignore-in-parent-frame #inbox-html-wrapper { color: rgb(1, 2, 3); }' +
      '\n.ignore-in-parent-frame a { color: rgb(4, 5, 6); }';
    store._findStyles();

    expect(store.themeColors()).toEqual({ text: 'rgb(1, 2, 3)', link: 'rgb(4, 5, 6)' });
  });
});
