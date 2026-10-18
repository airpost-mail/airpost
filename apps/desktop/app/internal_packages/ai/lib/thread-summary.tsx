// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import React from 'react';
import { AI, CapabilityStore, localized, Message, Thread } from 'mailspring-exports';
import { AiCall, errorMessage, isCancelled, summarize, destinationNote } from './ai-client';

// A single message this long (HTML) is worth summarizing on its own.
export const LONG_BODY_CHARS = 4000;
const MAX_REMEMBERED = 100;

interface Summary {
  text: string;
  model: string;
}

// The summaries this window showed, so opening a thread again shows its summary without a call.
// Keyed by the thread and its newest message: a new message makes it stale. Memory only (A7, Q7).
const shown = new Map<string, Summary>();

export function rememberedSummary(key: string) {
  return shown.get(key) || null;
}

function remember(key: string, summary: Summary) {
  shown.delete(key);
  shown.set(key, summary);
  while (shown.size > MAX_REMEMBERED) shown.delete(shown.keys().next().value);
}

export function forgetSummaries() {
  shown.clear();
}

function readable(messages: Message[]) {
  return (messages || []).filter((m) => !m.draft);
}

/** Threads of 2 or more messages, or one long one. */
export function worthSummarizing(messages: Message[]) {
  const sent = readable(messages);
  if (sent.length >= 2) return true;
  return sent.length === 1 && (sent[0].body || '').length >= LONG_BODY_CHARS;
}

export function summaryKey(thread: Thread, messages: Message[]) {
  const sent = readable(messages);
  const last = sent[sent.length - 1];
  return `${thread.id}:${last ? last.id : ''}`;
}

interface Props {
  thread: Thread;
  messages: Message[];
}

interface State {
  status: 'idle' | 'streaming' | 'done' | 'failed';
  text: string;
  model: string;
  error: string;
}

const IDLE: State = { status: 'idle', text: '', model: '', error: '' };

/** The Summarize button and the summary card above the first message (MessageListHeaders). */
export class ThreadSummary extends React.Component<Props, State> {
  static displayName = 'ThreadSummary';
  static containerRequired = false;

  _call: AiCall<unknown> | null = null;
  _unlisten?: () => void;
  _mounted = false;

  constructor(props: Props) {
    super(props);
    this.state = this._initialState(props);
  }

  componentDidMount() {
    this._mounted = true;
    this._unlisten = CapabilityStore.listen(() => this.forceUpdate());
  }

  componentDidUpdate(prev: Props) {
    if (
      summaryKey(prev.thread, prev.messages) !== summaryKey(this.props.thread, this.props.messages)
    ) {
      this._stop();
      this.setState(this._initialState(this.props));
    }
  }

  componentWillUnmount() {
    this._mounted = false;
    this._stop();
    if (this._unlisten) this._unlisten();
  }

  _initialState(props: Props): State {
    const remembered = rememberedSummary(summaryKey(props.thread, props.messages));
    return remembered ? { ...IDLE, status: 'done', ...remembered } : IDLE;
  }

  _stop() {
    if (this._call) this._call.cancel();
    this._call = null;
  }

  _onSummarize = (refresh = false) => {
    const { thread, messages } = this.props;
    const key = summaryKey(thread, messages);
    this._stop();
    this.setState({ status: 'streaming', text: '', model: '', error: '' });
    let streamed = '';
    const call = summarize(thread.accountId, thread.id, {
      refresh,
      onDelta: (text) => {
        streamed += text;
        if (this._mounted && this._call === call) this.setState({ text: streamed });
      },
    });
    this._call = call;
    return call.promise.then(
      (result) => {
        if (this._call !== call) return;
        this._call = null;
        remember(key, { text: result.text, model: result.model });
        if (this._mounted) {
          this.setState({ status: 'done', text: result.text, model: result.model });
        }
      },
      (err) => {
        if (this._call !== call) return;
        this._call = null;
        if (!this._mounted) return;
        if (isCancelled(err)) {
          this.setState(IDLE);
          return;
        }
        const provider = AI.providerFor(AI.readSettings(), 'summary');
        this.setState({
          status: 'failed',
          error: errorMessage(err, provider ? AI.providerName(provider) : undefined),
        });
      }
    );
  };

  _onStop = () => {
    this._stop();
    this.setState(IDLE);
  };

  _onClose = () => {
    this._stop();
    shown.delete(summaryKey(this.props.thread, this.props.messages));
    this.setState(IDLE);
  };

  /** Why the button is disabled, or null. */
  _unavailableReason() {
    const capability = CapabilityStore.get('ai-summary');
    if (!capability.available) return CapabilityStore.unavailableMessage('ai-summary');
    if (!AI.accountEnabled(AI.readSettings(), this.props.thread.accountId)) {
      return localized('Turn on AI for this account in Preferences > AI');
    }
    return null;
  }

  _renderCard(provider) {
    const { status, text, model, error } = this.state;
    const where = destinationNote(provider);
    return (
      <div className={`ai-summary-card ${status}`}>
        <div className="ai-summary-title">{localized('Summary')}</div>
        {status === 'failed' ? (
          <div className="ai-summary-error">{error}</div>
        ) : (
          <div className="ai-summary-text">
            {text || (status === 'streaming' ? localized('Summarizing…') : '')}
          </div>
        )}
        <div className="ai-summary-footer">
          <span className="ai-summary-source">{[model, where].filter(Boolean).join(' · ')}</span>
          {status === 'streaming' ? (
            <button className="btn btn-small" onClick={this._onStop}>
              {localized('Stop')}
            </button>
          ) : (
            <>
              <button className="btn btn-small" onClick={() => this._onSummarize(true)}>
                {localized('Regenerate')}
              </button>
              <button className="btn btn-small" onClick={this._onClose}>
                {localized('Close')}
              </button>
            </>
          )}
        </div>
      </div>
    );
  }

  render() {
    const settings = AI.readSettings();
    if (!settings.features.summary.enabled) return null;
    if (!worthSummarizing(this.props.messages)) return null;
    const provider = AI.providerFor(settings, 'summary');
    if (this.state.status !== 'idle') {
      return <div className="ai-thread-summary">{this._renderCard(provider)}</div>;
    }
    const reason = this._unavailableReason();
    return (
      <div className="ai-thread-summary">
        <button
          className="btn btn-small ai-summarize-button"
          disabled={!!reason}
          title={reason || destinationNote(provider)}
          onClick={() => this._onSummarize(false)}
        >
          {localized('Summarize')}
        </button>
      </div>
    );
  }
}
