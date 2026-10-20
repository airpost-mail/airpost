import React from 'react';
import { localized, Actions } from 'mailspring-exports';
import { IconButton } from '../../components/kit/icon-button';

export default class ComposeButton extends React.Component {
  static displayName = 'ComposeButton';

  _onNewCompose = () => {
    Actions.composeNewBlankDraft();
  };

  render() {
    return (
      <IconButton
        variant="toolbar"
        className="item-compose"
        icon="toolbar-compose.png"
        label={localized('Compose new message')}
        onClick={this._onNewCompose}
      />
    );
  }
}
