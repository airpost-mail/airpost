import classNames from 'classnames';
import React from 'react';
import * as Utils from '../../data/models/utils';
import DraftStore from '../../data/drafts/store';
import ComponentRegistry from '../../packages/component-registry';
import { Thread } from '../../data/models/thread';
import { Message } from '../../data/models/message';

import MessageItem from './message-item';

interface MessageItemContainerProps {
  thread: Thread;
  message: Message;
  messages: Message[];
  collapsed: boolean;
  isMostRecent: boolean;
  isBeforeReplyArea: boolean;
  scrollTo: () => void;
}

interface MessageItemContainerState {
  isSending: boolean;
}

export default class MessageItemContainer extends React.Component<
  MessageItemContainerProps,
  MessageItemContainerState
> {
  static displayName = 'MessageItemContainer';

  _unlisten?: () => void;
  // The rendered MessageItem or Composer; either may implement focus().
  _messageComponent: (React.Component & { focus?: () => void }) | null = null;

  constructor(props: MessageItemContainerProps, context: object) {
    super(props, context);
    this.state = this._getStateFromStores();
  }

  componentDidMount() {
    if (this.props.message.draft) {
      this._unlisten = DraftStore.listen(this._onSendingStateChanged);
    }
  }

  componentDidUpdate(prevProps: MessageItemContainerProps) {
    if (prevProps.message !== this.props.message || prevProps.thread !== this.props.thread) {
      this.setState(this._getStateFromStores(this.props));
    }
  }

  shouldComponentUpdate(
    nextProps: MessageItemContainerProps,
    nextState: MessageItemContainerState
  ) {
    return !Utils.isEqualReact(nextProps, this.props) || !Utils.isEqualReact(nextState, this.state);
  }

  componentWillUnmount() {
    if (this._unlisten) {
      this._unlisten();
    }
  }

  focus = () => {
    const component = this._messageComponent;
    if (component && component.focus) component.focus();
  };

  _classNames() {
    return classNames({
      draft: this.props.message.draft,
      unread: this.props.message.unread,
      collapsed: this.props.collapsed,
      'message-item-wrap': true,
      'before-reply-area': this.props.isBeforeReplyArea,
    });
  }

  _onSendingStateChanged = ({ headerMessageId }: { headerMessageId?: string }) => {
    if (headerMessageId === this.props.message.headerMessageId) {
      this.setState(this._getStateFromStores());
    }
  };

  _getStateFromStores(props = this.props) {
    return {
      isSending: DraftStore.isSendingDraft(props.message.headerMessageId),
    };
  }

  _renderMessage({ pending }: { pending: boolean }) {
    return (
      <MessageItem
        ref={(cm) => {
          this._messageComponent = cm;
        }}
        pending={pending}
        thread={this.props.thread}
        message={this.props.message}
        messages={this.props.messages}
        className={this._classNames()}
        collapsed={this.props.collapsed}
        isMostRecent={this.props.isMostRecent}
      />
    );
  }

  _renderComposer() {
    const Composer = ComponentRegistry.findComponentsMatching({ role: 'Composer' })[0];
    if (!Composer) {
      return <span>No Composer Component Present</span>;
    }
    return (
      <Composer
        ref={(cm: (React.Component & { focus?: () => void }) | null) => {
          this._messageComponent = cm;
        }}
        headerMessageId={this.props.message.headerMessageId}
        className={this._classNames()}
        mode={'inline'}
        threadId={this.props.thread.id}
        scrollTo={this.props.scrollTo}
      />
    );
  }

  render() {
    if (this.state.isSending) {
      return this._renderMessage({ pending: true });
    }
    if (this.props.message.draft && !this.props.collapsed) {
      return this._renderComposer();
    }
    return this._renderMessage({ pending: false });
  }
}
