import React from 'react';
import ReactDOM from 'react-dom';
import * as Actions from '../../data/actions';
import { localized } from '../../intl/localize';
import BindGlobalCommands from '../../components/bind-global-commands';
import { IconButton } from '../../components/kit/icon-button';
import { QuickEventPopover } from './quick-event-popover';

export class QuickEventButton extends React.Component<Record<string, unknown>> {
  static displayName = 'QuickEventButton';

  _openPopover = () => {
    const el = ReactDOM.findDOMNode(this) as HTMLElement;
    if (!el) return;
    const buttonRect = el.getBoundingClientRect();
    Actions.openPopover(<QuickEventPopover />, { originRect: buttonRect, direction: 'down' });
  };

  onClick = (event: React.MouseEvent) => {
    event.stopPropagation();
    this._openPopover();
  };

  render() {
    return (
      <BindGlobalCommands commands={{ 'core:add-item': this._openPopover }}>
        <IconButton
          variant="toolbar"
          tabIndex={-1}
          className="item-compose"
          icon="toolbar-compose.png"
          label={localized('Create new event')}
          onClick={this.onClick}
        />
      </BindGlobalCommands>
    );
  }
}
