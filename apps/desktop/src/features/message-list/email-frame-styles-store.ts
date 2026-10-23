import BaseStore from '../../data/stores/base';
import { EMAIL_RENDER_MODE_KEY } from './email-render-mode';

class EmailFrameStylesStore extends BaseStore {
  _styles?: string;
  _themeColors?: { text: string; link: string };
  _mutationObserver?: MutationObserver;
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

  // The text and link colours email-frame.css gives a message body under the current theme.
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

    // The email iframe is its own document, so it gets the theme tokens, the system accent and the
    // core variables that email-frame.css reads, in the order ThemeManager loaded them, and any
    // email-frame stylesheet (a theme may ship one, in LESS or CSS).
    const variablesPath = AppEnv.themes.coreVariablesPath();
    for (const sheet of Array.from(document.querySelectorAll<HTMLElement>('[source-path]'))) {
      const sourcePath = sheet.getAttribute('source-path') || '';
      if (
        sourcePath.startsWith('tokens:') ||
        sourcePath === 'system-accent:dynamic' ||
        sourcePath === variablesPath ||
        sourcePath.includes('email-frame.less') ||
        sourcePath.includes('email-frame.css')
      ) {
        this._styles += `\n${sheet.innerText}`;
      }
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
