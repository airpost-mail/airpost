import React from 'react';
import classNames from 'classnames';
import { localized } from '../../intl/localize';
import { RetinaImg } from '../../components/retina-img';
import { UnsubscribeOption, performWebUnsubscribe, performEngineUnsubscribe } from './service';

type UnsubscribeMethod = 'one-click' | 'mailto' | 'web' | 'body-link';

interface UnsubscribeHeaderProps {
  /** The unsubscribe option to use */
  option: UnsubscribeOption;
  /** The method to use for unsubscribing */
  method: UnsubscribeMethod;
  /** The message, for `messages.unsubscribe`. */
  message: { accountId: string; id: string };
}

type UnsubscribeState = 'idle' | 'loading' | 'success' | 'error';

interface UnsubscribeHeaderState {
  state: UnsubscribeState;
  errorMessage?: string;
}

/**
 * Displays an unsubscribe link in the message header area.
 *
 * The header methods (one-click, mailto, web) go through the engine's `messages.unsubscribe`:
 * it makes the RFC 8058 POST or sends the mailto message itself, and returns web links to
 * open. A link found only in the body opens in the browser.
 */
export class UnsubscribeHeader extends React.Component<
  UnsubscribeHeaderProps,
  UnsubscribeHeaderState
> {
  static displayName = 'UnsubscribeHeader';

  private _mounted = false;

  state: UnsubscribeHeaderState = {
    state: 'idle',
  };

  componentDidMount() {
    this._mounted = true;
  }

  componentWillUnmount() {
    this._mounted = false;
  }

  componentDidUpdate(prevProps: UnsubscribeHeaderProps) {
    // Reset state when the unsubscribe option changes (e.g., navigating between messages)
    if (prevProps.option.uri !== this.props.option.uri) {
      this.setState({ state: 'idle', errorMessage: undefined });
    }
  }

  private _handleClick = async () => {
    // Prevent multiple clicks while an operation is in progress
    if (this.state.state !== 'idle') {
      return;
    }

    const { option, method, message } = this.props;

    if (method === 'body-link') {
      this._performWeb(option.uri);
    } else {
      await this._performEngine(message);
    }
  };

  private async _performEngine(message: { accountId: string; id: string }) {
    this.setState({ state: 'loading', errorMessage: undefined });
    const result = await performEngineUnsubscribe(message);
    if (!this._mounted) return;
    if (!result.success) {
      this.setState({ state: 'error', errorMessage: result.error });
    } else {
      // A web link opened in the browser; the list isn't done with us until the page is.
      this.setState({ state: result.method === 'web' ? 'idle' : 'success' });
    }
  }

  private _performWeb(url: string) {
    try {
      performWebUnsubscribe(url);
    } catch (err) {
      console.error('Failed to open unsubscribe URL:', err);
    }
  }

  private _handleRetry = () => {
    this.setState({ state: 'idle', errorMessage: undefined });
  };

  render() {
    const { state } = this.state;

    const className = classNames('unsubscribe-action', {
      'unsubscribe-loading': state === 'loading',
      'unsubscribe-success': state === 'success',
      'unsubscribe-error': state === 'error',
    });

    if (state === 'loading') {
      return (
        <span className={className}>
          <RetinaImg
            name="inline-loading-spinner.gif"
            mode={RetinaImg.Mode.ContentPreserve}
            className="unsubscribe-spinner"
          />
          {localized('Unsubscribing...')}
        </span>
      );
    }

    if (state === 'success') {
      return <span className={className}>{localized('Unsubscribed')}</span>;
    }

    if (state === 'error') {
      return (
        <span className={className}>
          <span className="unsubscribe-error-message">{localized('Unsubscribe failed')}</span>
          <a className="unsubscribe-retry" onClick={this._handleRetry}>
            {localized('Try again')}
          </a>
        </span>
      );
    }

    // Idle state - show clickable link
    return (
      <a className={className} onClick={this._handleClick}>
        {localized('Unsubscribe')}
      </a>
    );
  }
}

export default UnsubscribeHeader;
