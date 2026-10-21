import React from 'react';
import ReactDOM from 'react-dom';

import {
  localized,
  Message,
  MessageViewExtension,
  getCurrentLocale,
  MessageBodyProcessor,
  CapabilityStore,
} from 'mailspring-exports';

import {
  translateMessageBody,
  isTranslationAvailable,
  translationDestination,
  AllLanguages,
} from './service';
import {
  RetinaImg,
  Button,
  DropdownMenuRoot,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from 'mailspring-component-kit';
import { messageFrameContent } from '../../../src/mail/frame/bridge';

interface TranslateMessageHeaderProps {
  message: Message;
}

interface TranslateMessageHeaderState {
  detected: string | null;
  translating: 'manual' | 'auto' | false;
}

let RecentlyTranslatedBodies: {
  id: string;
  enabled: boolean;
  fromLang: string | null;
  toLang: string;
}[] = [];

try {
  RecentlyTranslatedBodies = JSON.parse(window.localStorage.getItem('translated-index') || '[]');
} catch (err) {
  // no saved translations
}

/** The app's language code, e.g. "en"; localization is set up before any message renders. */
function currentLanguage() {
  const locale = getCurrentLocale();
  if (locale === null) throw new Error('initializeLocalization() has not run');
  return locale.split('-')[0];
}

function getPrefs(): { disabled: string[]; automatic: string[] } {
  return {
    disabled: AppEnv.config.get('core.translation.disabled') || [],
    automatic: AppEnv.config.get('core.translation.automatic') || [],
  };
}

function setPrefs(opts: { disabled: string[]; automatic: string[] }) {
  AppEnv.config.set('core.translation.disabled', opts.disabled);
  AppEnv.config.set('core.translation.automatic', opts.automatic);
}

export class TranslateMessageExtension extends MessageViewExtension {
  static formatMessageBody = ({ message }: { message: Message }) => {
    // retrieve from cache and push to the end to ensure the least recently viewed message is
    // removed from the cache first.
    const idx = RecentlyTranslatedBodies.findIndex((o) => o.id === message.id);
    if (idx === -1) return;

    const [result] = RecentlyTranslatedBodies.splice(idx, 1);
    RecentlyTranslatedBodies.push(result);

    if (result.enabled) {
      // A missing stored translation leaves the original body (it used to blank it with null).
      const translated = window.localStorage.getItem(`translated-${message.id}`);
      if (translated !== null) message.body = translated;
    }
  };
}

type CldResult = { languages: { language: string }[] };

function callCldViaExtension(
  text: string,
  callback: (err: string | null, result: CldResult | null) => void
) {
  const listener = (message: MessageEvent<any>) => {
    let resp: { response: string; text: string; result: CldResult | null; error: string | null };
    try {
      resp = JSON.parse(message.data);
    } catch (err) {
      return; // probably not a message for us
    }
    if (resp.response === 'detectLanguage' && resp.text === text) {
      window.removeEventListener('message', listener);
      callback(resp.error, resp.result);
    }
  };
  window.addEventListener('message', listener);
  window.postMessage(JSON.stringify({ call: 'detectLanguage', text }), '*');
}

export class TranslateMessageHeader extends React.Component<
  TranslateMessageHeaderProps,
  TranslateMessageHeaderState
> {
  static displayName = 'TranslateMessageHeader';

  _mounted = false;
  _detectionStarted = false;
  _unlisten?: () => void;

  state: TranslateMessageHeaderState = {
    detected: null,
    translating: false,
  };

  componentDidMount() {
    this._mounted = true;
    // Detection waits for a translation provider; start it once one is set up.
    this._unlisten = CapabilityStore.listen(() => this._detectLanguageIfReady());
    this._detectLanguageIfReady();
  }

  componentDidUpdate() {
    this._detectLanguageIfReady();
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._unlisten) this._unlisten();
  }

  _detectLanguageIfReady = async () => {
    if (this._detectionStarted) return;

    // we do not translate messages that YOU sent, because you can probably read them.
    if (this.props.message.isFromMe()) return;

    // load the previous translation result if this message is already translated
    const result = RecentlyTranslatedBodies.find((o) => o.id === this.props.message.id);
    if (result) {
      this._detectionStarted = true;
      this.setState({ detected: result.fromLang });
      return;
    }

    if (!isTranslationAvailable()) return;

    // add a delay to avoid this work if the user is rapidly flipping through messages
    await Promise.delay(1000);

    if (this._detectionStarted || !this._mounted) return;

    // we need to trim the quoted text, convert the HTML to plain text to analyze, etc.
    // the second step is costly and we can just wait for the message to mount and read
    // the innerText which is much more efficient.
    const el = ReactDOM.findDOMNode(this) as Element;
    const messageEl = el && el.closest('.message-item-area');
    const iframeEl = messageEl && messageEl.querySelector('iframe');
    if (!iframeEl || !this.props.message.body) return;
    // The message frame is cross-origin; it sends its text.
    const content = await messageFrameContent(iframeEl);
    if (!content || this._detectionStarted || !this._mounted) return;

    let text = content.text;
    if (text.length > 1000) text = text.slice(0, 1000);
    if (!text) return;

    if (text.length < 50) {
      // language detection seems unreliably for very short "hello world" emails
      return;
    }

    this._detectionStarted = true;

    callCldViaExtension(text, (err, result) => {
      if (err || !result || !result.languages?.length) {
        console.warn(`Could not detect message language: ${err && err.toString()}`);
        return;
      }
      if (!this._mounted) {
        return;
      }

      // no-op if the current and detected language are the same
      const detected = result.languages[0].language;
      const current = currentLanguage();
      if (current === detected) return;

      // no-op if we don't know how to translate this language pair
      if (!AllLanguages[current] || !AllLanguages[detected]) return;

      const prefs = getPrefs();
      if (prefs.disabled.includes(detected)) return;
      this.setState({ detected });
      if (prefs.automatic.includes(detected)) {
        this._onTranslate('auto');
      }
    });
  };

  _onTranslate = async (mode: 'auto' | 'manual') => {
    const { message } = this.props;

    const result = RecentlyTranslatedBodies.find((o) => o.id === message.id);
    if (result) {
      if (!result.enabled) this._onToggleTranslate();
      return;
    }

    this.setState({ translating: mode });
    const targetLanguage = currentLanguage();
    const translated = await translateMessageBody(
      message.body ?? '',
      targetLanguage,
      mode === 'auto'
    );
    if (this._mounted) {
      this.setState({ translating: false });
    }
    if (translated) {
      this._onPersistTranslation(targetLanguage, translated);
    }
  };

  _onPersistTranslation = (targetLanguage: string, translated: string) => {
    const { message } = this.props;

    if (RecentlyTranslatedBodies.length > 150) {
      const element = RecentlyTranslatedBodies.shift();
      if (element) localStorage.removeItem(`translated-${element.id}`);
    }

    RecentlyTranslatedBodies.push({
      id: message.id,
      enabled: true,
      fromLang: this.state.detected,
      toLang: targetLanguage,
    });
    localStorage.setItem(`translated-${message.id}`, translated);
    localStorage.setItem(`translated-index`, JSON.stringify(RecentlyTranslatedBodies));

    MessageBodyProcessor.updateCacheForMessage(message);
  };

  _onToggleTranslate = () => {
    const result = RecentlyTranslatedBodies.find((o) => o.id === this.props.message.id);
    if (!result) return;
    result.enabled = !result.enabled;
    MessageBodyProcessor.updateCacheForMessage(this.props.message);
  };

  _onDisableAlwaysForLanguage = () => {
    const prefs = getPrefs();
    prefs.automatic = prefs.automatic.filter((p) => p !== this.state.detected);
    setPrefs(prefs);
    this.forceUpdate();
  };

  _onAlwaysForLanguage = () => {
    const { detected } = this.state;
    if (!detected) return;
    const prefs = getPrefs();
    prefs.disabled = prefs.disabled.filter((p) => p !== detected);
    prefs.automatic = prefs.automatic.concat([detected]);
    setPrefs(prefs);

    this.forceUpdate();
    this._onTranslate('manual');
  };

  _onNeverForLanguage = () => {
    if (!this.state.detected) return;

    const response = require('@electron/remote').dialog.showMessageBoxSync({
      type: 'warning',
      buttons: [localized('Yes'), localized('Cancel')],
      message: localized('Are you sure?'),
      detail: localized(
        'Tern will no longer offer to translate messages written in %@.',
        AllLanguages[this.state.detected]
      ),
    });
    if (response === 0) {
      const prefs = getPrefs();
      prefs.disabled = prefs.disabled.concat([this.state.detected]);
      prefs.automatic = prefs.automatic.filter((p) => p !== this.state.detected);
      setPrefs(prefs);
      this.setState({ detected: null });
    }
  };

  _onReset = () => {
    setPrefs({ automatic: [], disabled: [] });
    this.forceUpdate();
  };

  render() {
    const result = RecentlyTranslatedBodies.find((o) => o.id === this.props.message.id);

    if (result && result.enabled) {
      return (
        <div className="translate-message-header">
          <div className="message with-actions">
            <div className="message-centered">
              {localized('Tern has translated this message into %@.', AllLanguages[result.toLang])}
            </div>
          </div>
          <div className="actions">
            <Button variant="ghost" className="action" onClick={this._onToggleTranslate}>
              <span>{localized('Show Original')}</span>
            </Button>
          </div>
        </div>
      );
    }

    if (!this.state.detected) {
      return <span />;
    }

    const fromLanguage = AllLanguages[this.state.detected];
    const toLanguage = AllLanguages[currentLanguage()];
    const prefs = getPrefs();

    const spinner = (
      <RetinaImg
        name="inline-loading-spinner.gif"
        mode={RetinaImg.Mode.ContentDark}
        style={{ width: 14, height: 14, mixBlendMode: 'multiply' }}
      />
    );

    if (this.state.translating === 'auto') {
      return (
        <div className="translate-message-header">
          <div className="message">
            <div className="message-centered">
              {localized('Translating from %1$@ to %2$@.', fromLanguage, toLanguage)}
            </div>
            <div style={{ flex: 1 }} />
            <RetinaImg
              name="inline-loading-spinner.gif"
              mode={RetinaImg.Mode.ContentDark}
              style={{ width: 14, height: 14, mixBlendMode: 'multiply' }}
            />
          </div>
        </div>
      );
    }
    return (
      <div className="translate-message-header">
        <div className="message with-actions">
          <div className="message-centered">
            {localized('Translate from %1$@ to %2$@?', fromLanguage, toLanguage)}
            <div className="note">
              {localized(
                'Privacy note: the text below will be sent to %@.',
                translationDestination()
              )}
            </div>
          </div>
        </div>
        <div className="actions">
          <Button variant="ghost" className="action" onClick={() => this._onTranslate('manual')}>
            {this.state.translating === 'manual' ? spinner : <span>{localized('Translate')}</span>}
          </Button>
          <DropdownMenuRoot>
            <DropdownMenuTrigger asChild>
              {/* data-slot is set again because the trigger's own data-slot would replace it. */}
              <Button variant="ghost" data-slot="button" className="action action-options">
                <span>{localized('Options')}</span>
                <RetinaImg name="icon-thread-disclosure.png" mode={RetinaImg.Mode.ContentIsMask} />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {prefs.automatic.includes(this.state.detected) ? (
                <DropdownMenuItem onSelect={this._onDisableAlwaysForLanguage}>
                  {localized('Stop translating %@', fromLanguage)}
                </DropdownMenuItem>
              ) : (
                <DropdownMenuItem onSelect={this._onAlwaysForLanguage}>
                  {localized('Always translate %@', fromLanguage)}
                </DropdownMenuItem>
              )}
              <DropdownMenuItem onSelect={this._onNeverForLanguage}>
                {localized('Never translate %@', fromLanguage)}
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={this._onReset}>
                {localized('Reset translation settings')}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenuRoot>
        </div>
      </div>
    );
  }
}
