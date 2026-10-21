import React from 'react';
import { localized } from '../../intl/localize';
import * as Actions from '../../data/actions';
import { IconButton } from '../../components/kit/icon-button';

export class DraftDeleteButton extends React.Component<{ selection: any }> {
  static displayName = 'DraftDeleteButton';
  static containerRequired = false;

  render() {
    return (
      <IconButton
        variant="toolbar"
        style={{ order: -100 }}
        label={localized('Delete')}
        icon="icon-composer-trash.png"
        onClick={this._onDestroySelected}
      />
    );
  }

  _onDestroySelected = () => {
    for (const item of this.props.selection.items()) {
      Actions.destroyDraft(item);
    }
    this.props.selection.clear();
    return;
  };
}
