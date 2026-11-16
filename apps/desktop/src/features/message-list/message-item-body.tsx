import { escapeHTML } from 'underscore.string';

import React from 'react';
import * as Utils from '../../data/models/utils';
import MessageUtils from '../../data/models/message-utils';
import MessageBodyProcessor from '../../mail/body/processor';
import QuotedHTMLTransformer from '../../mail/body/quoted-html-transformer';
import AttachmentStore from '../../data/stores/attachment';
import { Message } from '../../data/models/message';
import { InjectedComponentSet } from '../../components/injected/component-set';
import { RetinaImg } from '../../components/retina-img';

import EmailFrame from './email-frame';
import {
  contentSecurityPolicyMetaTag,
  messageContentSecurityPolicy,
} from '../../mail/content-security-policy';
import { openStaticPage } from '../../platform/app-api';
import { writePrivateTempFile } from '../../utils/private-temp';

const TransparentPixel =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR4nGNikAQAACIAHF/uBd8AAAAASUVORK5CYII=';

const SpinnerImg =
  '<img alt="spinner.gif" src="airpost-asset://message-list/assets/spinner.gif" style="-webkit-user-drag: none;">';

class ConditionalQuotedTextControl extends React.Component<{ body: string; onClick?: () => void }> {
  static displayName = 'ConditionalQuotedTextControl';

  shouldComponentUpdate(nextProps: { body: string; onClick?: () => void }) {
    return this.props.body !== nextProps.body;
  }

  render() {
    if (!QuotedHTMLTransformer.hasQuotedHTML(this.props.body)) {
      return null;
    }
    return (
      <a className="quoted-text-control" onClick={this.props.onClick}>
        <span className="dots">&bull;&bull;&bull;</span>
      </a>
    );
  }
}

interface MessageItemBodyProps {
  message: Message;
  downloads: any;
}

interface MessageItemBodyState {
  processedBody: string | null;
  clipped: boolean;
  remoteImages: boolean;
  showQuotedText: boolean;
}

export default class MessageItemBody extends React.Component<
  MessageItemBodyProps,
  MessageItemBodyState
> {
  static displayName = 'MessageItemBody';

  _mounted = false;
  _unsub?: () => void;

  constructor(props: MessageItemBodyProps, context: object) {
    super(props, context);

    const cached = MessageBodyProcessor.retrieveCached(props.message);

    this.state = {
      showQuotedText: props.message.isForwarded(),
      processedBody: cached ? cached.body : null,
      clipped: cached ? cached.clipped : false,
      remoteImages: cached ? cached.remoteImages : false,
    };
  }

  componentDidMount() {
    this._mounted = true;
    const needInitialCallback = this.state.processedBody === null;
    this._unsub = MessageBodyProcessor.subscribe(
      this.props.message,
      needInitialCallback,
      this._onBodyProcessed
    );
  }

  componentDidUpdate(prevProps: MessageItemBodyProps) {
    if (this.props.message.id !== prevProps.message.id) {
      if (this._unsub) {
        this._unsub();
      }
      this._unsub = MessageBodyProcessor.subscribe(this.props.message, true, this._onBodyProcessed);
    }
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._unsub) {
      this._unsub();
    }
  }

  _onBodyProcessed = ({
    body,
    clipped,
    remoteImages,
  }: {
    body: string;
    clipped: boolean;
    remoteImages: boolean;
  }) => this.setState({ processedBody: body, clipped, remoteImages });

  _onToggleQuotedText = () => {
    this.setState({
      showQuotedText: !this.state.showQuotedText,
    });
  };

  _onShowClipped = async () => {
    const { message } = this.props;
    // The full body gets the same sanitizing and formatting as the reading pane; only the
    // length clip is skipped.
    const { body, remoteImages } = await MessageBodyProcessor.processUnclipped(message);
    const html = message.plaintext
      ? `<div style="white-space: pre-wrap">${escapeHTML(body)}</div>`
      : this._mergeBodyWithFiles(body);
    const csp = contentSecurityPolicyMetaTag(messageContentSecurityPolicy({ remoteImages }));
    // The charset tag keeps non-ASCII text from rendering garbled
    const filepath = writePrivateTempFile(
      `${message.id}.html`,
      `${csp}\n<meta charset="UTF-8">\n${html}`
    );
    openStaticPage({ filePath: filepath, title: `${message.subject}`, width: 800, height: 600 });
  };

  _mergeBodyWithFiles(body: string) {
    let merged = body;

    // Replace cid: references with the paths to downloaded files
    this.props.message.files
      .filter((f) => f.contentId)
      .forEach((file) => {
        const download = this.props.downloads[file.id];
        const safeContentId = Utils.escapeRegExp(file.contentId);

        // Note: I don't like doing this with RegExp before the body is inserted into
        // the DOM, but we want to avoid "could not load cid://" in the console.
        const inlineImgRegexp = new RegExp(
          `<\\s*img[^>/]*src=['"]cid:${safeContentId}['"][^>]*>`,
          'gi'
        );

        if (download && download.state !== 'finished') {
          // Render a spinner
          merged = merged.replace(inlineImgRegexp, () => SpinnerImg);
        } else {
          merged = merged.replace(inlineImgRegexp, (match) => {
            const fileURL = AttachmentStore.fileSchemeURLForFile(file);
            if (!fileURL) return match;
            return match.replace(`cid:${file.contentId}`, () => fileURL);
          });
        }
      });

    // Replace remaining cid: references - we will not display them since they'll
    // throw "unknown ERR_UNKNOWN_URL_SCHEME". Show a transparent pixel so that there's
    // no "missing image" region shown, just a space.
    merged = merged.replace(MessageUtils.cidRegex, `src="${TransparentPixel}"`);

    return merged;
  }

  _renderBody() {
    const { message } = this.props;
    const { showQuotedText, processedBody } = this.state;

    if (typeof message.body === 'string' && typeof processedBody === 'string') {
      return (
        <EmailFrame
          showQuotedText={showQuotedText}
          content={this._mergeBodyWithFiles(processedBody)}
          message={message}
          remoteImages={this.state.remoteImages}
        />
      );
    }

    return (
      <div className="message-body-loading">
        <RetinaImg
          name="inline-loading-spinner.gif"
          mode={RetinaImg.Mode.ContentDark}
          style={{ width: 14, height: 14 }}
        />
      </div>
    );
  }

  render() {
    return (
      <span>
        <InjectedComponentSet
          matching={{ role: 'message:BodyHeader' }}
          exposedProps={{ message: this.props.message }}
          direction="column"
          style={{ width: '100%' }}
        />
        {this._renderBody()}
        <ConditionalQuotedTextControl
          body={this.props.message.body || ''}
          onClick={this._onToggleQuotedText}
        />
        {this.state.clipped && <a onClick={this._onShowClipped}>[Message Clipped - Show All]</a>}
      </span>
    );
  }
}
