import React from 'react';
import { Actions, ReactDOM, localized } from 'mailspring-exports';
import {
  RetinaImg,
  Button,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from 'mailspring-component-kit';
import { ownKeys } from '../../../../src/components/kit/keys';

import ActivityList from './list';
import ActivityEventStore from '../event-store';

// Space and Enter click the button and don't also reach the keymap.
const ownClickKeys = ownKeys([' ', 'Enter']);

class ActivityListButton extends React.Component<
  Record<string, unknown>,
  { unreadCount: number | string }
> {
  static displayName = 'ActivityListButton';

  _unsub: () => void;

  constructor(props) {
    super(props);
    this.state = this._getStateFromStores();
  }

  componentDidMount() {
    this._unsub = ActivityEventStore.listen(this._onDataChanged);
  }

  componentWillUnmount() {
    this._unsub();
  }

  onClick = () => {
    const buttonRect = (ReactDOM.findDOMNode(this) as HTMLElement).getBoundingClientRect();
    Actions.openPopover(<ActivityList />, { originRect: buttonRect, direction: 'down' });
  };

  _onDataChanged = () => {
    this.setState(this._getStateFromStores());
  };

  _getStateFromStores() {
    return {
      unreadCount: ActivityEventStore.unreadCount(),
    };
  }

  render() {
    let unreadCountClass = 'unread-count';
    let iconClass = 'activity-toolbar-icon';
    if (this.state.unreadCount) {
      unreadCountClass += ' active';
      iconClass += ' unread';
    }
    const ariaLabel = this.state.unreadCount
      ? localized('View activity (%@)', this.state.unreadCount)
      : localized('View activity');
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          {/* The trigger's own data-slot would replace the button's. */}
          <Button
            data-slot="button"
            variant="ghost"
            className="toolbar-activity"
            aria-label={ariaLabel}
            onClick={this.onClick}
            onKeyDown={ownClickKeys}
          >
            <div className={unreadCountClass}>{this.state.unreadCount}</div>
            <RetinaImg
              name="icon-toolbar-activity.png"
              className={iconClass}
              mode={RetinaImg.Mode.ContentIsMask}
              aria-hidden="true"
            />
          </Button>
        </TooltipTrigger>
        <TooltipContent>{localized('View activity')}</TooltipContent>
      </Tooltip>
    );
  }
}

export default ActivityListButton;
