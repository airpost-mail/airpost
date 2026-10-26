import React from 'react';
import { Thread } from '../../data/models/thread';
import { Message } from '../../data/models/message';
import { localized } from '../../intl/localize';
import * as Utils from '../../data/models/utils';
import * as Actions from '../../data/actions';
import AttachmentStore from '../../data/stores/attachment';
import { MessageStore } from '../../data/stores/message';
import { RetinaImg } from '../../components/retina-img';
import { InjectedComponentSet } from '../../components/injected/component-set';
import InjectedComponent from '../../components/injected/component';

import MessageParticipants from './message-participants';
import MessageItemBody from './message-item-body';
import MessageTimestamp from './message-timestamp';
import MessageControls from './message-controls';
import MessageAttachments from '../attachments/message-attachments';
import { Button } from '../../components/kit/button';
import { IconButton } from '../../components/kit/icon-button';

interface MessageItemProps {
  thread: Thread;
  message: Message;
  messages: Message[];
  collapsed: boolean;
  pending: boolean;
  isMostRecent: boolean;
  className: string;
  // Collapses and expands the message outside the message list, which MessageStore does not follow.
  onToggleCollapsed?: (message: Message) => void;
}

interface MessageItemState {
  filePreviewPaths: { [fileId: string]: string };
  downloads: {
    [fileId: string]: null;
  };
  detailedHeaders: boolean;
}

export default class MessageItem extends React.Component<MessageItemProps, MessageItemState> {
  static displayName = 'MessageItem';

  _headerEl: HTMLElement | null = null;

  constructor(props: MessageItemProps, context: object) {
    super(props, context);

    const fileIds = this.props.message.fileIds();
    this.state = {
      // Holds the downloadData (if any) for all of our files. It's a hash
      // keyed by a fileId. The value is the downloadData.
      downloads: AttachmentStore.getDownloadDataForFiles(fileIds),
      filePreviewPaths: AttachmentStore.previewPathsForFiles(fileIds),
      detailedHeaders: AppEnv.config.get('core.reading.detailedHeaders'),
    };
  }

  private _storeUnlisten?: () => void;

  componentDidMount() {
    this._storeUnlisten = AttachmentStore.listen(this._onDownloadStoreChange);
  }

  shouldComponentUpdate(nextProps: MessageItemProps, nextState: MessageItemState) {
    return !Utils.isEqualReact(nextProps, this.props) || !Utils.isEqualReact(nextState, this.state);
  }

  componentWillUnmount() {
    if (this._storeUnlisten) {
      this._storeUnlisten();
    }
  }

  _onClickParticipants = (e: React.MouseEvent<any>) => {
    let el: HTMLElement | null = e.target as HTMLElement;
    while (el && el !== e.currentTarget) {
      if (el.classList.contains('collapsed-participants')) {
        this.setState({ detailedHeaders: true });
        e.stopPropagation();
        return;
      }
      el = el.parentElement;
    }
    return;
  };

  _onClickHeader = (e: React.MouseEvent<HTMLElement>) => {
    this._onToggleCollapsed();
  };

  _onDownloadAll = () => {
    Actions.fetchAndSaveAllFiles(this.props.message.files);
  };

  _onToggleCollapsed = () => {
    if (this.props.isMostRecent) {
      return;
    }
    if (this.props.onToggleCollapsed) {
      this.props.onToggleCollapsed(this.props.message);
      return;
    }
    Actions.toggleMessageIdExpanded(this.props.message.id);
  };

  _onDownloadStoreChange = () => {
    const fileIds = this.props.message.fileIds();
    this.setState({
      downloads: AttachmentStore.getDownloadDataForFiles(fileIds),
      filePreviewPaths: AttachmentStore.previewPathsForFiles(fileIds),
    });
  };

  _renderDownloadAllButton() {
    return (
      <div className="download-all">
        <div className="attachment-number">
          <RetinaImg name="ic-attachments-all-clippy.png" mode={RetinaImg.Mode.ContentIsMask} />
          <span>{`${this.props.message.files.length} ${localized('attachments')}`}</span>
        </div>
        <div className="separator">-</div>
        <Button
          variant="ghost"
          size="sm"
          className="download-all-action"
          onClick={this._onDownloadAll}
        >
          <RetinaImg name="ic-attachments-download-all.png" mode={RetinaImg.Mode.ContentIsMask} />
          <span>{localized('Download All')}</span>
        </Button>
      </div>
    );
  }

  _renderAttachments() {
    const { files = [], body, headerMessageId } = this.props.message;
    const { filePreviewPaths, downloads } = this.state;
    let attachedFiles = files.filter(
      (f) => !f.contentId || !(body || '').includes(`cid:${f.contentId}`)
    );

    for (const extension of MessageStore.extensions()) {
      if (!extension.filterMessageFiles) continue;
      attachedFiles = extension.filterMessageFiles({
        message: this.props.message,
        files: attachedFiles,
      });
    }

    return (
      <div>
        {files.length > 1 ? this._renderDownloadAllButton() : null}
        {attachedFiles.length > 0 && (
          <div className="attachments-area">
            <MessageAttachments
              files={attachedFiles}
              headerMessageId={headerMessageId}
              downloads={downloads}
              filePreviewPaths={filePreviewPaths}
              canRemoveAttachments={false}
            />
          </div>
        )}
      </div>
    );
  }

  _renderFooterStatus() {
    return (
      <InjectedComponentSet
        className="message-footer-status"
        matching={{ role: 'MessageFooterStatus' }}
        exposedProps={{
          message: this.props.message,
          thread: this.props.thread,
          detailedHeaders: this.state.detailedHeaders,
        }}
      />
    );
  }

  _renderHeader() {
    const { message, thread, messages, pending } = this.props;

    return (
      <header
        ref={(el) => (this._headerEl = el)}
        className={`message-header ${pending && 'pending'}`}
        onClick={this._onClickHeader}
      >
        <InjectedComponent
          matching={{ role: 'MessageHeader' }}
          exposedProps={{ message: message, thread: thread, messages: messages }}
        />
        <div className="pending-spinner" style={{ position: 'absolute', marginTop: -2 }}>
          <RetinaImg width={18} name="sending-spinner.gif" mode={RetinaImg.Mode.ContentPreserve} />
        </div>
        <div className="message-header-right">
          <MessageTimestamp
            className="message-time"
            isDetailed={this.state.detailedHeaders}
            date={message.date}
          />
          <InjectedComponentSet
            className="message-header-status"
            matching={{ role: 'MessageHeaderStatus' }}
            exposedProps={{
              message: message,
              thread: thread,
              detailedHeaders: this.state.detailedHeaders,
            }}
          />
          <MessageControls thread={thread} message={message} />
        </div>
        <MessageParticipants
          from={message.from}
          onClick={this._onClickParticipants}
          isDetailed={this.state.detailedHeaders}
        />
        <MessageParticipants
          to={message.to}
          cc={message.cc}
          bcc={message.bcc}
          replyTo={message.replyTo.filter((c) => !message.from.find((fc) => fc.email === c.email))}
          onClick={this._onClickParticipants}
          isDetailed={this.state.detailedHeaders}
        />
        {this._renderSubject()}
        {this._renderFolder()}
        {this._renderHeaderDetailToggle()}
      </header>
    );
  }

  _renderHeaderDetailToggle() {
    if (this.props.pending) {
      return null;
    }
    const detailed = this.state.detailedHeaders;
    return (
      <IconButton
        className={detailed ? 'header-toggle-control' : 'header-toggle-control inactive'}
        size="sm"
        icon={
          detailed ? 'message-disclosure-triangle-active.png' : 'message-disclosure-triangle.png'
        }
        label={detailed ? localized('Hide Details') : localized('Show Details')}
        tabIndex={-1}
        onClick={(e) => {
          this.setState({ detailedHeaders: !detailed });
          e.stopPropagation();
        }}
      />
    );
  }

  _renderSubject() {
    if (!this.state.detailedHeaders) {
      return false;
    }

    const subject = this.props.message.subject;

    return (
      <div className="header-row">
        <div className="header-label">{localized('Subject')}:&nbsp;</div>
        <div className="header-name">{subject}</div>
      </div>
    );
  }

  _renderFolder() {
    if (!this.state.detailedHeaders) {
      return false;
    }

    const folders = this.props.message.categories().filter((f) => f.role !== 'all');
    if (folders.length === 0) {
      return false;
    }

    return (
      <div className="header-row">
        <div className="header-label">{localized('Folder')}:&nbsp;</div>
        <div className="header-name">
          {folders.map((f) => (
            <span key={f.id} className="message-folder-chip">
              {f.displayName}
            </span>
          ))}
        </div>
      </div>
    );
  }

  _renderCollapsed() {
    const {
      message: { snippet, from, files, date, draft },
      className,
    } = this.props;

    const attachmentIcon = Utils.showIconForAttachments(files) ? (
      <div className="collapsed-attachment" />
    ) : null;

    return (
      <div className={className} onClick={this._onToggleCollapsed}>
        <div className="message-item-white-wrap">
          <div className="message-item-area">
            <div className="collapsed-from">
              {from &&
                from[0] &&
                from[0].displayName({ compact: !AppEnv.config.get('core.reading.detailedNames') })}
            </div>
            <div className="collapsed-snippet">{snippet}</div>
            {draft && (
              <div className="Collapsed-draft">
                <RetinaImg
                  name="icon-draft-pencil.png"
                  className="draft-icon"
                  mode={RetinaImg.Mode.ContentPreserve}
                />
              </div>
            )}
            <div className="collapsed-timestamp">
              <MessageTimestamp date={date} />
            </div>
            {attachmentIcon}
          </div>
        </div>
      </div>
    );
  }

  _renderFull() {
    const senderName =
      this.props.message.from && this.props.message.from[0]
        ? this.props.message.from[0].displayName()
        : localized('Unknown');
    return (
      <article
        className={this.props.className}
        aria-label={localized('Message from %@', senderName)}
      >
        <div className="message-item-white-wrap">
          <div className="message-item-area">
            {this._renderHeader()}
            <MessageItemBody message={this.props.message} downloads={this.state.downloads} />
            {this._renderAttachments()}
            {this._renderFooterStatus()}
          </div>
        </div>
      </article>
    );
  }

  render() {
    return this.props.collapsed ? this._renderCollapsed() : this._renderFull();
  }
}
