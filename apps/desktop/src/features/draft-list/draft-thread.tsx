import React from 'react';
import classNames from 'classnames';
import * as Actions from '../../data/actions';
import { Thread } from '../../data/models/thread';
import { Message } from '../../data/models/message';
import DatabaseStore from '../../data/database/store';
import { QuerySubscription } from '../../data/query/subscription';
import QuerySubscriptionPool from '../../data/query/subscription-pool';
import MessageItem from '../message-list/message-item';

interface DraftThreadProps {
  threadId: string;
  // Called once, after the thread's messages first render.
  onShown?: () => void;
}

interface DraftThreadState {
  thread: Thread | null;
  messages: Message[];
  expanded: { [messageId: string]: true };
}

let subscriptions = 0;

const byDate = (a: Message, b: Message) => a.date.getTime() - b.date.getTime();

/*
The sent and received messages of the thread a draft replies in, shown above it in the Drafts
reading pane. It follows the thread by itself rather than through MessageStore, which belongs
to the focused thread of the mailbox. Bodies load as for the message list: only for expanded
messages, which show collapsed until theirs arrives.
*/
export default class DraftThread extends React.Component<DraftThreadProps, DraftThreadState> {
  static displayName = 'DraftThread';

  state: DraftThreadState = { thread: null, messages: [], expanded: {} };

  _unsubscribe: (() => void) | null = null;
  _unmounted = false;
  _shown = false;
  _bodiesLoading = new Set<string>();
  // Messages that changed while their body was being read; the read may predate the change.
  _changedWhileLoading = new Set<string>();
  _bodiesRequested = new Set<string>();

  componentDidMount() {
    const { threadId } = this.props;
    const key = `draft-thread-${++subscriptions}`;
    const messages = new QuerySubscription<Message>(
      DatabaseStore.findAll<Message>(Message).where({ threadId })
    );
    const thread = new QuerySubscription<Thread>(DatabaseStore.find<Thread>(Thread, threadId));
    const offMessages = QuerySubscriptionPool.addPrivateSubscription(
      `${key}-messages`,
      messages,
      this._onMessages
    );
    const offThread = QuerySubscriptionPool.addPrivateSubscription(
      `${key}-thread`,
      thread,
      (result: Thread | null) => this.setState({ thread: result })
    );
    this._unsubscribe = () => {
      offMessages();
      offThread();
    };
  }

  componentDidUpdate() {
    if (!this._shown && this.state.thread && this.state.messages.length > 0) {
      this._shown = true;
      if (this.props.onShown) this.props.onShown();
    }
  }

  componentWillUnmount() {
    this._unmounted = true;
    if (this._unsubscribe) this._unsubscribe();
  }

  _onMessages = (result: Message[]) => {
    // The rows come without bodies; keep the ones already read. A null body (not downloaded
    // yet) is read again, since the message's delta is what says it has arrived.
    const bodies = new Map(this.state.messages.map((m) => [m.id, m.body]));
    const messages = result
      .filter((m) => !m.draft && !m.isHidden() && !m.isInSpamOrTrashOnly())
      .sort(byDate)
      .map((m) => {
        const body = bodies.get(m.id);
        if (m.body !== undefined || typeof body !== 'string') return m;
        const withBody = new Message(m);
        withBody.body = body;
        return withBody;
      });
    messages
      .filter((m) => this._bodiesLoading.has(m.id))
      .forEach((m) => this._changedWhileLoading.add(m.id));

    let { expanded } = this.state;
    if (this.state.messages.length === 0 && messages.length > 0) {
      const last = messages[messages.length - 1];
      expanded = { [last.id]: true };
      this._fetchFiles(last);
    }
    this.setState({ messages, expanded }, this._loadExpandedBodies);
  };

  _onToggleCollapsed = (message: Message) => {
    const expanded = { ...this.state.expanded };
    if (expanded[message.id]) {
      delete expanded[message.id];
    } else {
      expanded[message.id] = true;
      this._fetchFiles(message);
    }
    this.setState({ expanded }, this._loadExpandedBodies);
  };

  _fetchFiles(message: Message) {
    message.files.forEach((file) => Actions.fetchFile(file));
  }

  _loadExpandedBodies = () => {
    const ids = this.state.messages
      .filter(
        (m) => this.state.expanded[m.id] && m.body === undefined && !this._bodiesLoading.has(m.id)
      )
      .map((m) => m.id);
    if (ids.length === 0) return;
    ids.forEach((id) => this._bodiesLoading.add(id));

    DatabaseStore.findAll<Message>(Message, { id: ids })
      .include(Message.attributes.body)
      .then(
        (loaded) => {
          ids.forEach((id) => this._bodiesLoading.delete(id));
          if (this._unmounted) return;
          const bodies = new Map(loaded.map((m) => [m.id, m.body]));
          const stale = ids.filter((id) => this._changedWhileLoading.delete(id));
          stale.forEach((id) => bodies.delete(id));
          const messages = this.state.messages.map((m) => {
            if (m.body !== undefined || !bodies.has(m.id)) return m;
            const withBody = new Message(m);
            withBody.body = bodies.get(m.id);
            return withBody;
          });
          const missing = messages.filter(
            (m) => bodies.has(m.id) && m.body === null && !this._bodiesRequested.has(m.id)
          );
          missing.forEach((m) => this._bodiesRequested.add(m.id));
          if (missing.length > 0) Actions.fetchBodies(missing);
          this.setState({ messages }, stale.length > 0 ? this._loadExpandedBodies : undefined);
        },
        (err) => {
          ids.forEach((id) => {
            this._bodiesLoading.delete(id);
            this._changedWhileLoading.delete(id);
          });
          console.warn(`DraftThread: could not read message bodies: ${err}`);
        }
      );
  };

  render() {
    const { thread, messages, expanded } = this.state;
    if (!thread || messages.length === 0) {
      return null;
    }
    return (
      <div className="draft-preview-thread">
        {messages.map((message) => {
          const collapsed = !expanded[message.id] || message.body === undefined;
          return (
            <MessageItem
              key={message.id}
              thread={thread}
              message={message}
              messages={messages}
              collapsed={collapsed}
              pending={false}
              isMostRecent={false}
              onToggleCollapsed={this._onToggleCollapsed}
              className={classNames('message-item-wrap', {
                collapsed,
                unread: message.unread,
              })}
            />
          );
        })}
      </div>
    );
  }
}
