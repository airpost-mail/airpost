import React from 'react';
import { localized } from '../intl/localize';
import { Message } from '../data/models/message';
import { DraftEditingSession } from '../data/drafts/editing-session';
import { RetinaImg } from './retina-img';
import classnames from 'classnames';
import _ from 'underscore';

type MetadataComposerToggleButtonProps = {
  iconUrl?: string;
  iconName?: string;
  pluginId: string;
  pluginName: string;
  metadataEnabledValue: any;
  errorMessage: (...args: any[]) => any;
  draft: Message;
  session: DraftEditingSession;
  /** When set, the feature can't be turned on (eg. its server is missing) and this is the tooltip. */
  unavailableReason?: string;
};
type MetadataComposerToggleButtonState = {
  pending: boolean;
};

export default class MetadataComposerToggleButton extends React.Component<
  MetadataComposerToggleButtonProps,
  MetadataComposerToggleButtonState
> {
  static displayName = 'MetadataComposerToggleButton';

  constructor(props: MetadataComposerToggleButtonProps) {
    super(props);
    this.state = { pending: false };
  }

  componentDidMount() {
    this._applyDefault();
  }

  componentDidUpdate(prevProps: MetadataComposerToggleButtonProps) {
    // The feature became available while the composer was open (eg. the server connected).
    if (prevProps.unavailableReason && !this.props.unavailableReason) {
      this._applyDefault();
    }
  }

  _applyDefault() {
    if (!this.props.unavailableReason && this._isEnabledByDefault() && !this._isEnabled()) {
      this._setEnabled(true);
    }
  }

  _configKey() {
    return `plugins.${this.props.pluginId}.defaultOn`;
  }

  _isEnabled() {
    const { pluginId, draft, metadataEnabledValue } = this.props;
    const value = draft.metadataForPluginId(pluginId);
    return _.isEqual(value, metadataEnabledValue) || _.isMatch(value, metadataEnabledValue);
  }

  // Off until the user turns it on once: the toggles are open and link tracking, which a
  // server connecting shouldn't switch on for every new draft.
  _isEnabledByDefault() {
    return AppEnv.config.get(this._configKey()) === true;
  }

  async _setEnabled(enabled: boolean) {
    const { pluginId, session, metadataEnabledValue } = this.props;

    const metadataValue = enabled ? metadataEnabledValue : null;
    this.setState({ pending: true });

    try {
      session.changes.addPluginMetadata(pluginId, metadataValue);
    } catch (error) {
      AppEnv.config.set(this._configKey(), false);
      AppEnv.reportError(error);
      AppEnv.showErrorDialog({
        title: localized('Error'),
        message: this.props.errorMessage(error),
      });
    }

    this.setState({ pending: false });
  }

  _onClick = () => {
    if (this.state.pending) {
      return;
    }
    const nextEnabled = !this._isEnabled();
    if (nextEnabled && this.props.unavailableReason) {
      return;
    }
    AppEnv.config.set(this._configKey(), nextEnabled);
    this._setEnabled(nextEnabled);
  };

  render() {
    const enabled = this._isEnabled();
    const unavailable = !enabled && !!this.props.unavailableReason;

    const className = classnames({
      btn: true,
      'btn-toolbar': true,
      'btn-pending': this.state.pending,
      'btn-enabled': enabled,
    });

    const attrs: any = {};
    if (this.props.iconUrl) {
      attrs.url = this.props.iconUrl;
    } else if (this.props.iconName) {
      attrs.name = this.props.iconName;
    }

    let title = `${enabled ? localized('Disable') : localized('Enable')} ${this.props.pluginName}`;
    if (unavailable) {
      title = this.props.unavailableReason;
    } else if (this.props.unavailableReason) {
      // On, but it won't apply when sent; say why and leave it clickable to turn off.
      title = `${title}. ${this.props.unavailableReason}`;
    }

    return (
      <button
        className={className}
        onClick={this._onClick}
        disabled={unavailable}
        title={title}
        aria-label={title}
        tabIndex={-1}
      >
        <RetinaImg {...attrs} mode={RetinaImg.Mode.ContentIsMask} aria-hidden="true" />
      </button>
    );
  }
}
