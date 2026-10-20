import React, { Component } from 'react';
import ReactDOM from 'react-dom';
import { localized, Actions, FocusedPerspectiveStore, Thread } from 'mailspring-exports';
import {
  BindGlobalCommands,
  Button,
  IconButton,
  RovingTabIndexToolbar,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from 'mailspring-component-kit';
import SnoozePopover from './popover';

interface BoundingRect {
  height: number;
  width: number;
  top: number;
  bottom: number;
  left: number;
  right: number;
}

interface SnoozeButtonProps {
  threads: Thread[];
  direction: string;
  /** `toolbar` is the toolbar's icon button; `quick-action` is the thread row's hover action,
   * whose icon is the `action-snooze` class's background image. */
  variant: 'toolbar' | 'quick-action';
  getBoundingClientRect: (inst?: any) => BoundingRect;
}

class SnoozeButton extends Component<SnoozeButtonProps> {
  static defaultProps = {
    direction: 'down',
    variant: 'toolbar',
    getBoundingClientRect: (inst) =>
      (ReactDOM.findDOMNode(inst) as HTMLElement).getBoundingClientRect(),
  };

  onClick = (event?: React.MouseEvent<any>) => {
    if (event) {
      event.stopPropagation();
    }
    const { threads, direction, getBoundingClientRect } = this.props;
    const buttonRect = getBoundingClientRect(this);
    Actions.openPopover(<SnoozePopover threads={threads} />, {
      originRect: buttonRect,
      direction: direction,
    });
  };

  render() {
    const label = localized('Snooze');
    if (this.props.variant === 'quick-action') {
      return (
        <Tooltip>
          <TooltipTrigger asChild>
            {/* The trigger's own data-slot would replace the button's. */}
            <Button
              data-slot="button"
              variant="outline"
              aria-label={label}
              className="snooze-button action action-snooze"
              onClick={this.onClick}
            />
          </TooltipTrigger>
          <TooltipContent>{label}</TooltipContent>
        </Tooltip>
      );
    }
    return (
      <IconButton
        variant="toolbar"
        tabIndex={-1}
        label={label}
        icon="toolbar-snooze.png"
        className="snooze-button"
        onClick={this.onClick}
      />
    );
  }
}

export class QuickActionSnooze extends Component<{ thread: Thread }> {
  static displayName = 'QuickActionSnooze';

  static containerRequired = false;

  getBoundingClientRect = () => {
    // Grab the parent node because of the zoom applied to this button. If we
    // took this element directly, we'd have to divide everything by 2
    const element = ReactDOM.findDOMNode(this).parentNode as HTMLElement;
    const { height, width, top, bottom, left, right } = element.getBoundingClientRect();

    // The parent node is a bit too much to the left, lets adjust this.
    return { height, width, top, bottom, right, left: left + 5 };
  };

  render() {
    if (!FocusedPerspectiveStore.current().isInbox()) {
      return <span />;
    }
    return (
      <SnoozeButton
        direction="left"
        threads={[this.props.thread]}
        variant="quick-action"
        getBoundingClientRect={this.getBoundingClientRect}
      />
    );
  }
}

export class ToolbarSnooze extends Component<{ items: Thread[] }> {
  static displayName = 'ToolbarSnooze';

  static containerRequired = false;

  _btn: SnoozeButton;

  render() {
    if (!FocusedPerspectiveStore.current().isInbox()) {
      return <span />;
    }
    return (
      <RovingTabIndexToolbar label="Snooze" className="button-group">
        <BindGlobalCommands commands={{ 'core:snooze-item': () => this._btn.onClick() }}>
          <SnoozeButton threads={this.props.items} ref={(b) => (this._btn = b)} />
        </BindGlobalCommands>
      </RovingTabIndexToolbar>
    );
  }
}
