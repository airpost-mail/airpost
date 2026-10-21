import { localized } from '../../../intl/localize';
import React from 'react';
import Notification from '../../../components/notification';

export default class DevModeNotification extends React.Component<
  Record<string, unknown>,
  { inDevMode: boolean }
> {
  static displayName = 'DevModeNotification';

  constructor(props) {
    super(props);
    // Don't need listeners to update this, since toggling dev mode reloads
    // the entire window anyway
    this.state = {
      inDevMode: AppEnv.inDevMode(),
    };
  }

  render() {
    if (!this.state.inDevMode) {
      return <span />;
    }
    return (
      <Notification
        priority="0"
        title={localized('Tern is running in dev mode and may be slower!')}
      />
    );
  }
}
