// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import React from 'react';
import {
  AI,
  Actions,
  CapabilityStore,
  localized,
  useCapability,
  AiSuggestion,
  DraftEditingSession,
  Message,
} from 'mailspring-exports';
import {
  AiCall,
  draftReply,
  destinationNote,
  errorMessage,
  isCancelled,
  suggestReplies,
} from './ai-client';
import { insertReply, ReplySession } from './reply-insert';

interface Draftish {
  accountId: string;
  threadId?: string;
  replyToHeaderMessageId?: string;
  body?: string;
  plaintext?: boolean;
}

interface PanelProps {
  draft: Draftish;
  session: ReplySession;
  /** Called after the reply went into the body. */
  onInserted?: () => void;
}

interface PanelState {
  suggestions: AiSuggestion[];
  loading: boolean;
  instruction: string;
  writing: boolean;
  preview: string;
  error: string;
}

function providerName() {
  const provider = AI.providerFor(AI.readSettings(), 'reply');
  return provider ? AI.providerName(provider) : undefined;
}

/**
 * Up to 3 one-line suggestions and a field for what to say. Picking one streams a full reply
 * into the preview; when it is complete it goes into the body above the quoted text in one
 * change. Nothing is sent.
 */
export class ReplySuggestionsPanel extends React.Component<PanelProps, PanelState> {
  static displayName = 'ReplySuggestionsPanel';

  _mounted = false;
  _suggestCall: AiCall<unknown> | null = null;
  _draftCall: AiCall<unknown> | null = null;

  state: PanelState = {
    suggestions: [],
    loading: true,
    instruction: '',
    writing: false,
    preview: '',
    error: '',
  };

  componentDidMount() {
    this._mounted = true;
    this._loadSuggestions();
  }

  componentWillUnmount() {
    this._mounted = false;
    if (this._suggestCall) this._suggestCall.cancel();
    if (this._draftCall) this._draftCall.cancel();
  }

  _loadSuggestions() {
    const { draft } = this.props;
    const call = suggestReplies(draft.accountId, draft.threadId, 3);
    this._suggestCall = call;
    return call.promise.then(
      (suggestions) => {
        this._suggestCall = null;
        if (this._mounted) this.setState({ suggestions: suggestions.slice(0, 3), loading: false });
      },
      (err) => {
        this._suggestCall = null;
        if (!this._mounted || isCancelled(err)) return;
        this.setState({ loading: false, error: errorMessage(err, providerName()) });
      }
    );
  }

  _write(options: { suggestion?: string; instruction?: string }) {
    const { draft, session } = this.props;
    if (this._draftCall) this._draftCall.cancel();
    if (this._suggestCall) this._suggestCall.cancel();
    this.setState({ writing: true, preview: '', error: '', loading: false });
    let streamed = '';
    const call = draftReply(draft.accountId, draft.threadId, {
      ...options,
      onDelta: (text) => {
        streamed += text;
        if (this._mounted && this._draftCall === call) this.setState({ preview: streamed });
      },
    });
    this._draftCall = call;
    return call.promise.then(
      (result) => {
        if (this._draftCall !== call) return;
        this._draftCall = null;
        const text = (result.text || '').trim();
        if (this._mounted) this.setState({ writing: false, preview: text });
        if (!text) return;
        insertReply(session, text);
        if (this.props.onInserted) this.props.onInserted();
      },
      (err) => {
        if (this._draftCall !== call) return;
        this._draftCall = null;
        if (!this._mounted) return;
        this.setState({
          writing: false,
          error: isCancelled(err) ? '' : errorMessage(err, providerName()),
        });
      }
    );
  }

  _onPick = (suggestion: AiSuggestion) => this._write({ suggestion: suggestion.text });

  _onWrite = () => {
    const instruction = this.state.instruction.trim();
    if (!instruction) return;
    return this._write({ instruction });
  };

  _onStop = () => {
    if (this._draftCall) this._draftCall.cancel();
    this._draftCall = null;
    this.setState({ writing: false });
  };

  render() {
    const { suggestions, loading, instruction, writing, preview, error } = this.state;
    const provider = AI.providerFor(AI.readSettings(), 'reply');
    return (
      <div className="ai-reply-suggestions">
        {loading && <div className="ai-reply-status">{localized('Thinking of replies…')}</div>}
        {!writing && suggestions.length > 0 && (
          <ul className="ai-reply-list">
            {suggestions.map((s, i) => (
              <li key={i}>
                <button
                  className="btn ai-reply-suggestion"
                  title={s.text}
                  onClick={() => this._onPick(s)}
                >
                  {s.label || s.text}
                </button>
              </li>
            ))}
          </ul>
        )}
        {!writing && (
          <div className="ai-reply-instruction">
            <input
              type="text"
              className="ai-reply-instruction-input"
              value={instruction}
              placeholder={localized("What to say, e.g. say I'll call Tuesday")}
              onChange={(e) => this.setState({ instruction: e.target.value })}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  this._onWrite();
                }
              }}
            />
            <button className="btn" disabled={!instruction.trim()} onClick={this._onWrite}>
              {localized('Write')}
            </button>
          </div>
        )}
        {(writing || preview) && (
          <div className="ai-reply-preview">{preview || localized('Writing…')}</div>
        )}
        {writing && (
          <button className="btn btn-small" onClick={this._onStop}>
            {localized('Stop')}
          </button>
        )}
        {error && <div className="ai-reply-error">{error}</div>}
        <div className="ai-reply-source">{destinationNote(provider)}</div>
      </div>
    );
  }
}

export function isReplyDraft(draft: Draftish) {
  return !!(draft && draft.threadId && draft.replyToHeaderMessageId);
}

/** Why Suggest is disabled for this draft, or null. */
export function suggestUnavailableReason(accountId: string) {
  const capability = CapabilityStore.get('ai-reply');
  if (!capability.available) return CapabilityStore.unavailableMessage('ai-reply');
  if (!AI.accountEnabled(AI.readSettings(), accountId)) {
    return localized('Turn on AI for this account in Preferences > AI');
  }
  return null;
}

type ButtonProps = { draft: Message; session: DraftEditingSession };

const SuggestReplyButtonInner: React.FC<ButtonProps> = ({ draft, session }) => {
  const buttonRef = React.useRef<HTMLButtonElement>(null);
  // Re-renders when the settings or the engine change.
  useCapability('ai-reply');
  if (!AI.readSettings().features.reply.enabled || !isReplyDraft(draft)) return <span />;
  const reason = suggestUnavailableReason(draft.accountId);

  const onClick = () => {
    if (!buttonRef.current) return;
    Actions.openPopover(
      <ReplySuggestionsPanel
        draft={session.draft()}
        session={session}
        onInserted={() => Actions.closePopover()}
      />,
      { originRect: buttonRef.current.getBoundingClientRect(), direction: 'up' }
    );
  };

  const title = reason || localized('Suggest a reply');
  return (
    <button
      ref={buttonRef}
      tabIndex={-1}
      className="btn btn-toolbar btn-ai-suggest pull-right"
      onClick={onClick}
      disabled={!!reason}
      title={title}
      aria-label={title}
    >
      {localized('Suggest')}
    </button>
  );
};

// The draft changes on every keystroke; the button only needs the session.
export const SuggestReplyButton = React.memo(
  SuggestReplyButtonInner,
  (prev, next) => prev.session === next.session
);
SuggestReplyButton.displayName = 'SuggestReplyButton';
