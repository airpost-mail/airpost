import React from 'react';
import { localized, Message, DraftEditingSession } from 'mailspring-exports';
import { RetinaImg } from 'mailspring-component-kit';
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

  constructor(props) {
    super(props);
    this.state = { pending: false };
  }

  componentDidMount() {
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

  _isEnabledByDefault() {
    return AppEnv.config.get(this._configKey()) !== false;
  }

  async _setEnabled(enabled) {
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

    const title = unavailable
      ? this.props.unavailableReason
      : `${enabled ? localized('Disable') : localized('Enable')} ${this.props.pluginName}`;

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
