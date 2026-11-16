import { localized } from '../../../intl/localize';
import React from 'react';
import { ipcRenderer, shell } from 'electron';
import Notification from '../../../components/notification';
import { Disposable } from 'event-kit';
import { App } from '../../../platform/app-api';

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
  mounted = false;

  // Nothing shows until main answers with the updater's state, just after mount.
  state: UpdateNotificationState = { updateAvailable: false, updateIsManual: false, version: '' };

  componentDidMount() {
    this.mounted = true;
    this.refresh();
    this.disposable = AppEnv.onUpdateAvailable(() => {
      this.refresh();
    });
  }

  componentWillUnmount() {
    this.mounted = false;
    if (this.disposable) this.disposable.dispose();
  }

  async refresh() {
    const next = await this.getStateFromStores();
    if (this.mounted) this.setState(next);
  }

  async getStateFromStores(): Promise<UpdateNotificationState> {
    const updater = await App.updaterState();
    const updateAvailable = updater.state === 'update-available';
    return {
      updateAvailable,
      updateIsManual: updateAvailable && updater.releaseNotes === 'manual-download',
      version: (updateAvailable && updater.releaseVersion) || '',
    };
  }

  _onUpdate = () => {
    ipcRenderer.send('command', 'application:install-update');
  };

  _onViewChangelog = () => {
    shell.openExternal('https://github.com/airpost-mail/airpost/releases');
  };

  render() {
    const { updateAvailable, version, updateIsManual } = this.state;

    if (!updateAvailable) {
      return <span />;
    }
    return (
      <Notification
        priority="4"
        title={localized(`An update to Airpost is available %@`, version ? `(${version.trim()})` : '')}
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
