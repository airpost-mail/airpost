import { localized } from '../../../intl/localize';
import React from 'react';
import { ipcRenderer, shell } from 'electron';
import Notification from '../../../components/notification';
import { Disposable } from 'event-kit';

interface UpdateNotificationState {
  updateAvailable: boolean;
  version: string;
  updateIsManual: boolean;
}

export default class UpdateNotification extends React.Component<
  Record<string, unknown>,
  UpdateNotificationState
> {
  static displayName = 'UpdateNotification';

  disposable?: Disposable;

  constructor(props: Record<string, unknown>) {
    super(props);
    this.state = this.getStateFromStores();
  }

  componentDidMount() {
    this.disposable = AppEnv.onUpdateAvailable(() => {
      this.setState(this.getStateFromStores());
    });
  }

  componentWillUnmount() {
    if (this.disposable) this.disposable.dispose();
  }

  getStateFromStores() {
    const updater = require('@electron/remote').getGlobal('application').autoUpdateManager;
    const updateAvailable = updater.getState() === 'update-available';
    const info = updateAvailable ? updater.getReleaseDetails() : {};
    return {
      updateAvailable,
      updateIsManual: info.releaseNotes === 'manual-download',
      version: info.releaseVersion,
    };
  }

  _onUpdate = () => {
    ipcRenderer.send('command', 'application:install-update');
  };

  _onViewChangelog = () => {
    shell.openExternal('https://github.com/ternmail/ternmail/releases');
  };

  render() {
    const { updateAvailable, version, updateIsManual } = this.state;

    if (!updateAvailable) {
      return <span />;
    }
    return (
      <Notification
        priority="4"
        title={localized(`An update to Tern is available %@`, version ? `(${version.trim()})` : '')}
        subtitle={localized('View changelog')}
        subtitleAction={this._onViewChangelog}
        icon="volstead-upgrade.png"
        actions={[
          {
            label: updateIsManual ? localized('Download Now') : localized('Install'),
            fn: this._onUpdate,
          },
        ]}
        isDismissable
      />
    );
  }
}
