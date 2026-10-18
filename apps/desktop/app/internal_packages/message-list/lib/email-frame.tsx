import { EventedIFrame } from 'mailspring-component-kit';
import React from 'react';
import ReactDOM from 'react-dom';
import {
  Utils,
  localized,
  QuotedHTMLTransformer,
  MessageStore,
  Message,
  Autolink,
} from 'mailspring-exports';
import { applyColorScheme, darkenDocument, RGBA } from '@tern/email-dark';
import {
  contentSecurityPolicyMetaTag,
  messageContentSecurityPolicy,
} from '../../../src/content-security-policy';
import { adjustImages } from './adjust-images';
import EmailFrameStylesStore from './email-frame-styles-store';
import { prepareEmailColors } from './email-color-detection';
import {
  OriginalColors,
  darkPalette,
  darkeningApplies,
  themeBackgroundAt,
} from './email-render-mode';

interface EmailFrameProps {
  content: string;
  showQuotedText: boolean;
  message: Message;
  // Whether this message may load remote images and fonts (MessageBodyProcessor decides).
  remoteImages?: boolean;
}

export default class EmailFrame extends React.Component<EmailFrameProps> {
  static displayName = 'EmailFrame';

  _mounted = false;
  _unlisten: () => void;
  _unlistenOriginalColors: () => void;
  _showingOriginal = false;
  _iframeComponent: EventedIFrame;
  _iframeWrapperEl: HTMLDivElement;
  _iframeDocObserver: ResizeObserver;
  _lastFitSize = '';

  componentDidMount() {
    this._mounted = true;
    this._iframeDocObserver = new window.ResizeObserver((entries) =>
      window.requestAnimationFrame(() => {
        if (!this._mounted) return;
        this._onReevaluateContentSize(entries[0]);
      })
    );
    this._writeContent();
    this._unlisten = EmailFrameStylesStore.listen(this._writeContent);
    this._unlistenOriginalColors = OriginalColors.listen(this._onOriginalColorsChanged);
  }

  shouldComponentUpdate(nextProps: EmailFrameProps) {
    const { content, showQuotedText, message, remoteImages } = this.props;
    const nextMessage = nextProps.message;

    return (
      (message ? message.id : '') !== (nextMessage ? nextMessage.id : '') ||
      content !== nextProps.content ||
      showQuotedText !== nextProps.showQuotedText ||
      !!remoteImages !== !!nextProps.remoteImages ||
      !Utils.isEqualReact(
        message && message.pluginMetadata,
        nextMessage && nextMessage.pluginMetadata
      )
    );
  }

  componentDidUpdate() {
    this._writeContent();
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._iframeDocObserver) this._iframeDocObserver.disconnect();
    if (this._unlisten) this._unlisten();
    if (this._unlistenOriginalColors) this._unlistenOriginalColors();
  }

  _onOriginalColorsChanged = () => {
    const { message } = this.props;
    if (message && OriginalColors.isShowingOriginal(message.id) !== this._showingOriginal) {
      this._writeContent();
    }
  };

  // Dark themes get the email's own dark variant if it has one, else a darkened copy of its
  // colours, unless the user chose the original colours. Otherwise transparent emails blend
  // into the theme and emails that paint their own backgrounds get the white page they were
  // designed for.
  _applyColors(
    iframeEl: HTMLIFrameElement,
    doc: Document,
    wrapper: HTMLElement,
    darken: boolean,
    themeBackground: RGBA
  ) {
    if (darken) {
      const { text, link } = EmailFrameStylesStore.themeColors();
      darkenDocument(doc, darkPalette(themeBackground, text, link), {
        root: wrapper,
        imageBrightness: 0.9,
      });
      return;
    }

    // prepareEmailColors can replace hardcoded black text with the inherited theme colour.
    const { r, g, b, a } = themeBackground;
    const hasBackground = prepareEmailColors(wrapper, [r, g, b, a]);
    wrapper.classList.toggle('has-background', hasBackground);
  }

  _writeContent = () => {
    if (this._iframeDocObserver) this._iframeDocObserver.disconnect();

    const iframeEl = ReactDOM.findDOMNode(this._iframeComponent) as HTMLIFrameElement;
    const doc = iframeEl.contentDocument;
    if (!doc) return;

    // NOTE: The iframe must have a modern DOCTYPE. The lack of this line
    // will cause some bizzare non-standards compliant rendering with the
    // message bodies. This is particularly felt with <table> elements use
    // the `border-collapse: collapse` css property while setting a
    // `padding`.
    const { message, showQuotedText } = this.props;
    const { themeStyles } = EmailFrameStylesStore.styles();
    const restrictWidth = AppEnv.config.get('core.reading.restrictMaxWidth');

    this._showingOriginal = OriginalColors.isShowingOriginal(message.id);
    const themeBackground = themeBackgroundAt(iframeEl);
    const darken = darkeningApplies(themeBackground) && !this._showingOriginal;

    let content = this.props.content;
    if (!showQuotedText && !message.plaintext) {
      content = QuotedHTMLTransformer.removeQuotedHTML(content, {
        keepIfWholeBodyIsQuote: true,
      });
    }

    // The policy must be the document's first element. A document keeps every policy it has
    // seen across doc.open(), so a change in remoteImages remounts the iframe (see render).
    const csp = contentSecurityPolicyMetaTag(
      messageContentSecurityPolicy({ remoteImages: !!this.props.remoteImages })
    );

    doc.open();

    if (message.plaintext) {
      doc.write(
        `<!DOCTYPE html>` +
          csp +
          `<style>${themeStyles}</style>` +
          `<div id='inbox-plain-wrapper' class="${process.platform}"></div>`
      );
      doc.close();
      applyColorScheme(iframeEl, darken ? 'dark' : 'light');
      const plainWrapper = doc.getElementById('inbox-plain-wrapper');
      plainWrapper.innerText = content;
      plainWrapper.setAttribute('role', 'document');
    } else {
      doc.write(
        `<!DOCTYPE html>` +
          csp +
          `<style>${themeStyles || ''}</style>` +
          `<div id='inbox-html-wrapper' class="${process.platform}">${content}</div>`
      );
      doc.close();
      // Before darkening, so an email's own `prefers-color-scheme: dark` rules are what it sees.
      applyColorScheme(iframeEl, darken ? 'dark' : 'light');
      const htmlWrapper = doc.getElementById('inbox-html-wrapper');
      if (htmlWrapper) {
        htmlWrapper.setAttribute('role', 'document');
        try {
          this._applyColors(iframeEl, doc, htmlWrapper, darken, themeBackground);
        } catch (e) {
          AppEnv.reportError(e);
        }
      }
    }

    if (doc.body && restrictWidth) {
      doc.body.classList.add('restrict-width');
    }

    // Notify the EventedIFrame that we've replaced it's document (with `open`)
    // so it can attach event listeners again.
    this._lastFitSize = '';
    this._iframeComponent.didReplaceDocument();

    // Observe the <html> element within the iFrame for changes to it's content
    // size. We need to disconnect the observer before the HTML element is deleted
    // or Chrome gets into a "maximum call depth" Observer error.
    const observedEl = iframeEl.contentDocument.firstElementChild;
    this._iframeDocObserver.observe(observedEl);
    iframeEl.contentWindow.addEventListener('beforeunload', () => {
      this._iframeDocObserver.disconnect();
    });

    window.requestAnimationFrame(() => {
      // Guard: component may have unmounted or _writeContent called again (calling
      // doc.open() a second time), leaving doc.body null until the next doc.write().
      if (!this._mounted || !doc.body) return;

      Autolink(doc.body, {
        async: true,
        telAggressiveMatch: false,
      });
      adjustImages(doc);

      for (const extension of MessageStore.extensions()) {
        if (!extension.renderedMessageBodyIntoDocument) {
          continue;
        }
        try {
          extension.renderedMessageBodyIntoDocument({
            document: doc,
            message: message,
            iframe: iframeEl,
          });
        } catch (e) {
          const name = (extension as any).displayName || (extension as any).name || 'unknown';
          AppEnv.reportError(e, { extensionName: name });
        }
      }
    });
  };

  _onReevaluateContentSize = (entry: ResizeObserverEntry) => {
    const size = `${entry.contentRect.width}:${entry.contentRect.height}`;
    if (size === this._lastFitSize || !this._iframeComponent) return;

    this._lastFitSize = size;
    const iframeEl = ReactDOM.findDOMNode(this._iframeComponent) as HTMLIFrameElement;
    const doc = iframeEl && iframeEl.contentDocument;

    // We must set the height to zero in order to get a valid scrollHeight
    // if the document is wider and has a lower height now.
    this._iframeComponent.setHeightQuietly(0);

    // If documentElement has a scroll height, prioritize that as height
    // If not, fall back to body scroll height by setting it to auto
    let height = 0;
    let width = 0;
    if (doc && doc.documentElement && doc.documentElement.scrollHeight > 0) {
      width = doc.documentElement.scrollWidth;
      height = doc.documentElement.scrollHeight;
    } else if (doc && doc.body) {
      const style = window.getComputedStyle(doc.body);
      if (style.height === '0px') {
        doc.body.style.height = 'auto';
      }
      width = doc.body.scrollWidth;
      height = doc.body.scrollHeight;
    }

    if (width > iframeEl.clientWidth) {
      // the message will scroll horizontally, and we need to add 20px to the height of
      // the iframe to allow for it's scrollbar. Otherwise it covers the last line of text.
      height += 20;
    }

    this._iframeComponent.setHeightQuietly(height);
    this._iframeWrapperEl.style.height = `${height}px`;
  };

  render() {
    return (
      <div
        className="message-iframe-container"
        style={{ height: 0 }}
        ref={(el) => {
          this._iframeWrapperEl = el;
        }}
      >
        <EventedIFrame
          key={this.props.remoteImages ? 'remote-images' : 'no-remote-images'}
          data-remote-images={this.props.remoteImages ? 'allowed' : 'blocked'}
          searchable
          sandbox="allow-forms allow-same-origin"
          seamless={true}
          title={localized('Email message')}
          style={{ height: 0 }}
          ref={(cm) => {
            this._iframeComponent = cm;
          }}
        />
      </div>
    );
  }
}
