import React from 'react';
import {
  localized,
  Message,
  DraftEditingSession,
  GrammarCheckPluginAPI,
  CapabilityStore,
} from 'mailspring-exports';
import classnames from 'classnames';
import {
  RetinaImg,
  Button,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from 'mailspring-component-kit';
import { GrammarCheckStore } from './store';

export class GrammarCheckToggle extends React.Component<{
  draft: Message;
  /** Passed by the Composer:ActionButton role; the toggle doesn't use it. */
  session?: DraftEditingSession | null;
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
    const icon = (
      <span className="grammar-check-icon-wrap">
        <RetinaImg
          url="tern-asset://composer-grammar-check/assets/icon-composer-grammar@2x.png"
          mode={RetinaImg.Mode.ContentIsMask}
          aria-hidden="true"
        />
        {this._checking() && <span className="grammar-check-spinner" />}
      </span>
    );

    if (!GrammarCheckStore.isAvailable()) {
      const reason = CapabilityStore.unavailableMessage('grammar-check');
      // A disabled button takes no pointer events, so the tooltip hangs off a wrapper.
      return (
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="grammar-check-unavailable">
              <Button
                variant="toolbar"
                tabIndex={-1}
                className="btn-grammar-check"
                disabled
                aria-label={reason ?? undefined}
              >
                {icon}
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>{reason}</TooltipContent>
        </Tooltip>
      );
    }

    const enabled = GrammarCheckStore.isEnabled();
    const errorCount = GrammarCheckStore.errorCount(this.props.draft.headerMessageId);

    let title: string;
    if (enabled) {
      title =
        errorCount > 0
          ? localized(`Grammar check: %@ issue(s)`, errorCount)
          : localized('Grammar check: no issues');
    } else {
      title = localized('Enable grammar check');
    }

    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            // TooltipTrigger's data-slot would otherwise replace the button's own.
            data-slot="button"
            variant="toolbar"
            tabIndex={-1}
            className={classnames('btn-grammar-check', {
              'btn-enabled': enabled,
              checking: this._checking(),
            })}
            aria-pressed={enabled}
            onClick={this._onClick}
            aria-label={title}
          >
            {icon}
          </Button>
        </TooltipTrigger>
        <TooltipContent>{title}</TooltipContent>
      </Tooltip>
    );
  }

  _checking() {
    return (
      GrammarCheckStore.isAvailable() &&
      GrammarCheckStore.isChecking(this.props.draft.headerMessageId)
    );
  }
}
