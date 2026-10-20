import React from 'react';
import {
  localized,
  Message,
  DraftEditingSession,
  GrammarCheckPluginAPI,
  CapabilityStore,
} from 'mailspring-exports';
import { RetinaImg } from 'mailspring-component-kit';
import { GrammarCheckStore } from './store';

export class GrammarCheckToggle extends React.Component<{
  draft: Message;
  session: DraftEditingSession;
}> {
  static displayName = 'GrammarCheckToggle';

  private _unsub?: () => void;

  componentDidMount() {
    this._unsub = GrammarCheckStore.listen(() => this.forceUpdate());
  }

  componentWillUnmount() {
    if (this._unsub) this._unsub();
  }

  _onClick = () => {
    const next = !AppEnv.config.get('core.composing.grammarCheck');
    AppEnv.config.set('core.composing.grammarCheck', next);
    if (next) {
      GrammarCheckPluginAPI.requestInitialCheckForDraft(this.props.draft.headerMessageId);
    }
  };

  render() {
    if (!GrammarCheckStore.isAvailable()) {
      const reason = CapabilityStore.unavailableMessage('grammar-check');
      return (
        <button
          tabIndex={-1}
          className="btn btn-toolbar btn-grammar-check"
          disabled
          title={reason}
          aria-label={reason}
        >
          <span className="grammar-check-icon-wrap">
            <RetinaImg
              url="mailspring://composer-grammar-check/assets/icon-composer-grammar@2x.png"
              mode={RetinaImg.Mode.ContentIsMask}
              aria-hidden="true"
            />
          </span>
        </button>
      );
    }

    const enabled = GrammarCheckStore.isEnabled();
    const checking = GrammarCheckStore.isChecking(this.props.draft.headerMessageId);
    const errorCount = GrammarCheckStore.errorCount(this.props.draft.headerMessageId);

    let title: string;
    let className = 'btn btn-toolbar btn-grammar-check';

    if (enabled) {
      className += ' btn-enabled';
    }
    if (checking) {
      className += ' checking';
    }

    if (enabled) {
      title =
        errorCount > 0
          ? localized(`Grammar check: %@ issue(s)`, errorCount)
          : localized('Grammar check: no issues');
    } else {
      title = localized('Enable grammar check');
    }

    return (
      <button
        tabIndex={-1}
        className={className}
        onClick={this._onClick}
        title={title}
        aria-label={title}
      >
        <span className="grammar-check-icon-wrap">
          <RetinaImg
            url="mailspring://composer-grammar-check/assets/icon-composer-grammar@2x.png"
            mode={RetinaImg.Mode.ContentIsMask}
            aria-hidden="true"
          />
          {checking && <span className="grammar-check-spinner" />}
        </span>
      </button>
    );
  }
}
