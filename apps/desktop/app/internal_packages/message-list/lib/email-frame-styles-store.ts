import MailspringStore from 'mailspring-store';
import { EMAIL_RENDER_MODE_KEY } from './email-render-mode';

class EmailFrameStylesStore extends MailspringStore {
  _styles?: string;
  _themeColors?: { text: string; link: string };
  _mutationObserver: MutationObserver;
  _configDisposable?: { dispose: () => void };

  constructor() {
    super();
    this._configDisposable = AppEnv.config.onDidChange(EMAIL_RENDER_MODE_KEY, this._findStyles);
  }

  styles() {
    if (!this._styles) {
      this._findStyles();
      this._listenToStyles();
    }
    return { themeStyles: this._styles };
  }

  // The text and link colours email-frame.less gives a message body under the current theme.
  themeColors() {
    if (!this._themeColors) {
      const scope = document.createElement('span');
      scope.className = 'ignore-in-parent-frame';
      const wrapper = document.createElement('div');
      wrapper.id = 'inbox-html-wrapper';
      const link = document.createElement('a');
      wrapper.appendChild(link);
      scope.appendChild(wrapper);
      document.body.appendChild(scope);
      this._themeColors = {
        text: window.getComputedStyle(wrapper).color,
        link: window.getComputedStyle(link).color,
      };
      scope.remove();
    }
    return this._themeColors;
  }

  _findStyles = () => {
    this._styles = '';
    this._themeColors = undefined;

    // The email iframe is its own document, so it gets the theme tokens and the system accent
    // that email-frame.less reads, in the order ThemeManager loaded them.
    for (const sheet of Array.from(
      document.querySelectorAll(
        '[source-path^="tokens:"], [source-path="system-accent:dynamic"], [source-path*="email-frame.less"]'
      )
    )) {
      this._styles += `\n${(sheet as HTMLElement).innerText}`;
    }
    this._styles = this._styles.replace(/.ignore-in-parent-frame/g, '');
    this.trigger();
  };

  _listenToStyles() {
    const target = document.getElementsByTagName('managed-styles')[0];
    this._mutationObserver = new MutationObserver(this._findStyles);
    this._mutationObserver.observe(target, { attributes: true, subtree: true, childList: true });
  }

  _unlistenToStyles() {
    if (this._mutationObserver) {
      this._mutationObserver.disconnect();
    }
    if (this._configDisposable) {
      this._configDisposable.dispose();
      this._configDisposable = undefined;
    }
  }
}

export { EmailFrameStylesStore };
export default new EmailFrameStylesStore();
