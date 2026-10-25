import React from 'react';
import {
  localized,
  // The ComponentRegistry manages all React components in N1.
  ComponentRegistry,
  // A `Store` is a Flux component which contains all business logic and data
  // models to be consumed by React components to render markup.
  MessageStore,
  Message,
  // The window's way to the sync engine.
  Engine,
} from 'mailspring-exports';

// The engine decides whether a message looks like phishing: its `warnings` (messages.get) have
// `reply-to-mismatch` when replies go to another registered domain than the message came from.
// This package shows that warning above the thread's first message.
const REPLY_TO_MISMATCH = 'reply-to-mismatch';

async function hasReplyToMismatch(message: Message) {
  const { message: json } = await Engine.request<{
    message: { warnings?: { code: string }[] };
  }>('messages.get', { accountId: message.accountId, id: message.id });
  return (json.warnings || []).some((w) => w.code === REPLY_TO_MISMATCH);
}

// Notice that this file is `main.tsx` rather than `main.ts`. We use the
// `.tsx` filetype because we use the TSX DSL to describe markup for React to
// render. Without the TSX, we could just name this file `main.ts` instead.
class PhishingIndicator extends React.Component<
  Record<string, unknown>,
  { message?: Message; suspicious: boolean }
> {
  // Adding a displayName to a React component helps for debugging.
  static displayName = 'PhishingIndicator';

  _unlisten?: () => void;
  _mounted = false;

  constructor(props: Record<string, unknown>) {
    super(props);
    this.state = { message: MessageStore.items()[0], suspicious: false };
  }

  componentDidMount() {
    this._mounted = true;
    this._unlisten = MessageStore.listen(this._onMessagesChanged);
    this._check(this.state.message);
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._unlisten) {
      this._unlisten();
    }
  }

  _onMessagesChanged = () => {
    const message = MessageStore.items()[0];
    if (message?.id === this.state.message?.id) return;
    this.setState({ message, suspicious: false });
    this._check(message);
  };

  _check(message?: Message) {
    if (!message || !message.replyTo?.length) return;
    hasReplyToMismatch(message).then(
      (suspicious) => {
        if (this._mounted && this.state.message?.id === message.id) {
          this.setState({ suspicious });
        }
      },
      (err) => console.warn(`phishing-detection: could not read ${message.id}: ${err}`)
    );
  }

  // A React component's `render` method returns a virtual DOM element described
  // in JSX. `render` is deterministic: with the same input, it will always
  // render the same output. Here, the input is `this.state`.
  render() {
    const { message, suspicious } = this.state;
    if (!message || !suspicious) {
      return <span />;
    }

    const fromEmail = (message.from[0]?.email || '').toLowerCase();
    const replyToEmail = (message.replyTo[0]?.email || '').toLowerCase();
    return (
      <div className="phishingIndicator">
        <b>{localized(`This message looks suspicious!`)}</b>
        <div className="description">
          {localized(`It originates from %@ but replies will go to %@.`, fromEmail, replyToEmail)}
        </div>
      </div>
    );
  }
}

export function activate() {
  ComponentRegistry.register(PhishingIndicator, {
    role: 'MessageListHeaders',
  });
}

export function serialize() {}

export function deactivate() {
  ComponentRegistry.unregister(PhishingIndicator);
}
