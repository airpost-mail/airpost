import React from 'react';
import {
  Utils,
  localized,
  QuotedHTMLTransformer,
  MessageStore,
  Message,
  SearchableComponentStore,
} from 'mailspring-exports';
import { confirmFormSubmission, showFrameContextMenu } from '../../components/evented-iframe';
import { MessageBody, MessageFrameBridge } from '../../mail/frame/bridge';
import {
  ForwardedKeyEvent,
  ForwardedMouseEvent,
  FrameToAppMessage,
  MessageFrameSandbox,
  SearchMatchRect,
  messageFrameURL,
} from '../../shared/message-frame-protocol';
import EmailFrameStylesStore from './email-frame-styles-store';
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

const warnedExtensions = new Set<string>();

class DocumentHookError extends Error {}

// The arguments of MessageViewExtension.renderedMessageBodyIntoDocument. `document` is gone
// since plugin API 1.27: the body is in a cross-origin frame, reached through `body`.
function hookArguments(body: MessageBody, message: Message, iframe: HTMLIFrameElement) {
  return {
    body,
    message,
    iframe,
    get document(): Document {
      throw new DocumentHookError(
        'renderedMessageBodyIntoDocument no longer gets the document (plugin API 1.27); use `body`'
      );
    },
  };
}

const finite = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : 0);

/**
 * Shows a message body in the sandboxed message frame (src/mail/frame): a cross-origin
 * frame in its own sandboxed renderer process, so nothing in an email (images, fonts, CSS)
 * is decoded in a process with Node. The frame renders what this component posts and posts
 * back its height, clicks, keys and find-in-thread matches.
 */
export default class EmailFrame extends React.Component<EmailFrameProps> {
  static displayName = 'EmailFrame';

  _mounted = false;
  _unlisten: () => void;
  _unlistenOriginalColors: () => void;
  _unlistenSearch: () => void;
  _showingOriginal = false;
  _iframeEl: HTMLIFrameElement | null = null;
  _iframeWrapperEl: HTMLDivElement;
  _bridge: MessageFrameBridge | null = null;
  _regionId = Utils.generateTempId();
  _searchMatches: SearchMatchRect[] = [];
  _lastSearch = '';

  componentDidMount() {
    this._mounted = true;
    this._writeContent();
    this._unlisten = EmailFrameStylesStore.listen(this._writeContent);
    this._unlistenOriginalColors = OriginalColors.listen(this._onOriginalColorsChanged);
    this._unlistenSearch = SearchableComponentStore.listen(this._onSearchableStoreChange);
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
    if (this._unlisten) this._unlisten();
    if (this._unlistenOriginalColors) this._unlistenOriginalColors();
    if (this._unlistenSearch) this._unlistenSearch();
    this._setIframe(null);
  }

  /** The frame's bridge, for specs and for code that reads the rendered body. */
  bridge() {
    return this._bridge;
  }

  // The frame element changes when remoteImages does: a document keeps the policy it loaded
  // with, so the other policy needs a new frame (see render).
  _setIframe = (el: HTMLIFrameElement | null) => {
    if (el === this._iframeEl) return;
    if (this._bridge) this._bridge.dispose();
    this._bridge = null;
    if (this._iframeEl) SearchableComponentStore.unregisterSearchRegion(this._regionId);
    this._iframeEl = el;
    this._searchMatches = [];
    this._lastSearch = '';
    if (el) {
      this._bridge = new MessageFrameBridge(el, this._onFrameMessage);
      this._registerSearchRegion();
      this._onSearchableStoreChange();
    }
  };

  _registerSearchRegion() {
    if (!this._iframeEl) return;
    SearchableComponentStore.registerSearchRegion(this._regionId, {
      element: this._iframeEl,
      matches: this._searchMatches,
    });
  }

  _onOriginalColorsChanged = () => {
    const { message } = this.props;
    if (message && OriginalColors.isShowingOriginal(message.id) !== this._showingOriginal) {
      this._writeContent();
    }
  };

  _writeContent = () => {
    const iframeEl = this._iframeEl;
    const bridge = this._bridge;
    if (!this._mounted || !iframeEl || !bridge) return;

    const { message, showQuotedText } = this.props;
    const { themeStyles } = EmailFrameStylesStore.styles();
    const restrictWidth = !!AppEnv.config.get('core.reading.restrictMaxWidth');

    // Dark themes get the email's own dark variant if it has one, else a darkened copy of
    // its colours, unless the user chose the original colours. Otherwise transparent emails
    // blend into the theme and emails that paint their own backgrounds get the white page
    // they were designed for.
    this._showingOriginal = OriginalColors.isShowingOriginal(message.id);
    const themeBackground = themeBackgroundAt(iframeEl);
    const darken = darkeningApplies(themeBackground) && !this._showingOriginal;
    let palette = null;
    if (darken && !message.plaintext) {
      const { text, link } = EmailFrameStylesStore.themeColors();
      palette = darkPalette(themeBackground, text, link);
    }

    let content = this.props.content;
    if (!showQuotedText && !message.plaintext) {
      content = QuotedHTMLTransformer.removeQuotedHTML(content, {
        keepIfWholeBodyIsQuote: true,
      });
    }

    // Chromium takes a frame's preferred colour scheme from the iframe element.
    iframeEl.style.colorScheme = darken ? 'dark' : 'light';
    bridge.post({
      type: 'render',
      generation: bridge.nextGeneration(),
      html: content || '',
      plaintext: !!message.plaintext,
      themeStyles: themeStyles || '',
      platform: process.platform,
      restrictWidth,
      colorScheme: darken ? 'dark' : 'light',
      darkPalette: palette,
      themeBackground,
    });
  };

  _runExtensions(generation: number) {
    const bridge = this._bridge;
    if (!bridge || !this._iframeEl) return;
    const body = new MessageBody(bridge, generation);
    for (const extension of MessageStore.extensions()) {
      if (!extension.renderedMessageBodyIntoDocument) {
        continue;
      }
      const name = (extension as any).displayName || (extension as any).name || 'unknown';
      const report = (e: Error) => {
        if (e instanceof DocumentHookError) {
          if (!warnedExtensions.has(name)) {
            warnedExtensions.add(name);
            console.warn(`${name}: ${e.message}`);
          }
          return;
        }
        // A newer render or an unmount rejects the older render's calls; that's expected.
        if (this._mounted && bridge.generation === generation) {
          AppEnv.reportError(e, { extensionName: name });
        }
      };
      try {
        const result = extension.renderedMessageBodyIntoDocument(
          hookArguments(body, this.props.message, this._iframeEl) as any
        ) as unknown;
        if (result && typeof (result as Promise<void>).then === 'function') {
          (result as Promise<void>).catch(report);
        }
      } catch (e) {
        report(e);
      }
    }
  }

  _setHeight(height: number) {
    if (!this._iframeEl || !this._iframeWrapperEl) return;
    const px = `${height}px`;
    if (this._iframeEl.style.height !== px) this._iframeEl.style.height = px;
    this._iframeWrapperEl.style.height = px;
  }

  // What the frame sends is the email's content as much as ours: check it before acting.
  _onFrameMessage = (msg: FrameToAppMessage) => {
    if (!this._mounted || !this._iframeEl) return;
    switch (msg.type) {
      case 'rendered':
        if (this._bridge && msg.generation === this._bridge.generation) {
          this._runExtensions(msg.generation);
        }
        break;
      case 'size':
        if (typeof msg.height === 'number' && Number.isFinite(msg.height) && msg.height >= 0) {
          this._setHeight(Math.round(msg.height));
        }
        break;
      case 'link':
        if (typeof msg.href === 'string' && msg.href && !/^file:/i.test(msg.href)) {
          AppEnv.windowEventHandler.openLink({ href: msg.href, metaKey: !!msg.metaKey });
        }
        break;
      case 'form': {
        const submission = {
          url: String(msg.url || ''),
          method: msg.method === 'post' ? ('post' as const) : ('get' as const),
        };
        if (/^https?:\/\//i.test(submission.url) && confirmFormSubmission(submission)) {
          AppEnv.windowEventHandler.openLink({ href: submission.url });
        }
        break;
      }
      case 'key':
        this._dispatchKey(msg.event);
        break;
      case 'mouse':
        this._dispatchMouse(msg.event);
        break;
      case 'contextmenu':
        showFrameContextMenu({
          href: typeof msg.href === 'string' ? msg.href : null,
          src: typeof msg.src === 'string' ? msg.src : null,
          text: typeof msg.text === 'string' ? msg.text : '',
        });
        break;
      case 'focus':
        window.getSelection().empty();
        break;
      case 'searchResults':
        if (msg.regionId === this._regionId && Array.isArray(msg.matches)) {
          const matches = msg.matches.map((m) => ({
            top: finite(m && m.top),
            left: finite(m && m.left),
            width: finite(m && m.width),
            height: finite(m && m.height),
            renderIndex: finite(m && m.renderIndex),
          }));
          if (!Utils.isEqual(matches, this._searchMatches)) {
            this._searchMatches = matches;
            this._registerSearchRegion();
          }
        }
        break;
      default:
        break;
    }
  };

  _dispatchKey(event: ForwardedKeyEvent) {
    if (!event || !['keydown', 'keypress', 'keyup'].includes(event.type)) return;
    const eventInParentDoc = new KeyboardEvent(event.type, {
      bubbles: true,
      key: String(event.key || ''),
      code: String(event.code || ''),
      location: finite(event.location),
      ctrlKey: !!event.ctrlKey,
      shiftKey: !!event.shiftKey,
      altKey: !!event.altKey,
      metaKey: !!event.metaKey,
      repeat: !!event.repeat,
      isComposing: !!event.isComposing,
      charCode: finite(event.charCode),
      keyCode: finite(event.keyCode),
    });
    Object.defineProperty(eventInParentDoc, 'which', { value: finite(event.which) });
    this._iframeEl.dispatchEvent(eventInParentDoc);
  }

  _dispatchMouse(event: ForwardedMouseEvent) {
    if (!event || !['mousedown', 'mousemove', 'mouseup'].includes(event.type)) return;
    const rect = this._iframeEl.getBoundingClientRect();
    this._iframeEl.dispatchEvent(
      new MouseEvent(event.type, {
        bubbles: true,
        button: finite(event.button),
        buttons: finite(event.buttons),
        detail: finite(event.detail),
        ctrlKey: !!event.ctrlKey,
        shiftKey: !!event.shiftKey,
        altKey: !!event.altKey,
        metaKey: !!event.metaKey,
        clientX: finite(event.clientX) + rect.left,
        clientY: finite(event.clientY) + rect.top,
        screenX: finite(event.screenX) + rect.left,
        screenY: finite(event.screenY) + rect.top,
      })
    );
  }

  _onSearchableStoreChange = () => {
    if (!this._bridge) return;
    const { searchTerm } = SearchableComponentStore.getCurrentSearchData();
    const currentIndex = SearchableComponentStore.getCurrentRegionIndex(this._regionId);
    const key = `${searchTerm}\u0000${currentIndex}`;
    if (key === this._lastSearch) return;
    this._lastSearch = key;
    this._bridge.post({ type: 'search', regionId: this._regionId, term: searchTerm, currentIndex });
  };

  render() {
    const remoteImages = !!this.props.remoteImages;
    return (
      <div
        className="message-iframe-container"
        style={{ height: 0 }}
        ref={(el) => {
          this._iframeWrapperEl = el;
        }}
      >
        <iframe
          key={remoteImages ? 'remote-images' : 'no-remote-images'}
          data-remote-images={remoteImages ? 'allowed' : 'blocked'}
          src={messageFrameURL({ remoteImages })}
          sandbox={MessageFrameSandbox}
          title={localized('Email message')}
          style={{ height: 0 }}
          ref={this._setIframe}
        />
      </div>
    );
  }
}
