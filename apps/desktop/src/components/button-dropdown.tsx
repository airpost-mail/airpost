import React, { CSSProperties } from 'react';
import ReactDOM from 'react-dom';
import { RetinaImg } from './retina-img';
import { localized } from '../intl/localize';
import classnames from 'classnames';
import { Popover, PopoverAnchor, PopoverContent } from './kit/popover';

const MENU_KEYS = ['ArrowUp', 'ArrowDown', 'Enter'];

type ButtonDropdownState = {
  open: 'up' | 'down' | false;
};

type ButtonDropdownProps = {
  bordered?: boolean;
  primaryTitle?: string;
  primaryItem: React.ReactElement;
  primaryClick?: () => void;
  menu: React.ReactElement;
  style?: CSSProperties;
  closeOnMenuClick?: boolean;
  attachment?: string;
  className?: string;
};

/* Public: A button with a menu: a primary item that acts and a disclosure that opens `menu`, or,
   without `primaryClick`, one item that opens it. The menu is the UI kit's Popover: it closes
   on a click outside, on Escape and when focus leaves, and flips up when there's no room
   below. It is rendered inside the component, as `.secondary-items`, so styles that reach it
   through `.button-dropdown` keep applying. */
export class ButtonDropdown extends React.Component<ButtonDropdownProps, ButtonDropdownState> {
  static displayName = 'ButtonDropdown';

  _root: HTMLDivElement | null = null;

  static defaultProps = {
    style: {},
    attachment: 'left',
  };

  constructor(props: ButtonDropdownProps) {
    super(props);
    this.state = { open: false };
  }

  render() {
    const classes = classnames({
      'button-dropdown': true,
      'open open-up': this.state.open === 'up',
      'open open-down': this.state.open === 'down',
      bordered: this.props.bordered !== false,
    });

    const { primaryClick } = this.props;
    if (primaryClick) {
      return this._renderAnchored(
        <div
          ref={this._setRoot}
          onBlur={this._onBlur}
          onKeyDown={this._onRootKeyDown}
          tabIndex={-1}
          className={`${classes} ${this.props.className || ''}`}
          style={this.props.style}
        >
          <div
            role="button"
            tabIndex={0}
            className="primary-item"
            title={this.props.primaryTitle || ''}
            aria-label={this.props.primaryTitle}
            onClick={primaryClick}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                primaryClick();
              }
            }}
          >
            {this.props.primaryItem}
          </div>
          <div
            role="button"
            tabIndex={0}
            aria-label={localized('More options')}
            aria-haspopup="menu"
            aria-expanded={this.state.open !== false}
            className="secondary-picker"
            onClick={this.toggleDropdown}
            onKeyDown={(e) => {
              if (this._forwardKeyToMenu(e)) return;
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                this.toggleDropdown();
              }
            }}
          >
            <RetinaImg name={'icon-thread-disclosure.png'} mode={RetinaImg.Mode.ContentIsMask} />
          </div>
        </div>,
        this._renderMenu('secondary-items', 'end', 'menu')
      );
    } else {
      return this._renderAnchored(
        <div
          ref={this._setRoot}
          onBlur={this._onBlur}
          onKeyDown={this._onRootKeyDown}
          tabIndex={-1}
          className={`${classes} ${this.props.className || ''}`}
          style={this.props.style}
        >
          <div
            className="only-item"
            title={this.props.primaryTitle || ''}
            onClick={this.toggleDropdown}
          >
            {this.props.primaryItem}
            <RetinaImg
              name={'icon-thread-disclosure.png'}
              style={{ marginLeft: 12 }}
              mode={RetinaImg.Mode.ContentIsMask}
            />
          </div>
        </div>,
        this._renderMenu(
          `secondary-items ${this.props.attachment}`,
          this.props.attachment === 'left' ? 'start' : 'end'
        )
      );
    }
  }

  _setRoot = (el: HTMLDivElement | null) => {
    this._root = el;
  };

  _renderAnchored(button: React.ReactElement, menu: React.ReactElement | null) {
    return (
      <Popover open={this.state.open !== false} onOpenChange={this._onOpenChange}>
        <PopoverAnchor asChild>{button}</PopoverAnchor>
        {menu}
      </Popover>
    );
  }

  _renderMenu(className: string, align: 'start' | 'end', role?: string) {
    if (this.state.open === false || !this._root) return null;
    return (
      <PopoverContent
        data-variant="plain"
        role={role}
        className={className}
        // Portalled into the component itself rather than the window's portal root.
        container={this._root}
        side={this.state.open === 'up' ? 'top' : 'bottom'}
        align={align}
        sideOffset={1}
        collisionPadding={4}
        onMouseDown={this._onMenuClick}
        // Focus stays where it was, as before the kit: the menu takes it only when clicked.
        onOpenAutoFocus={(e) => e.preventDefault()}
        onCloseAutoFocus={(e) => e.preventDefault()}
        onInteractOutside={(e) => {
          // The buttons toggle the menu themselves.
          if (this._root && this._root.contains(e.target as Node)) e.preventDefault();
        }}
      >
        {this.props.menu}
      </PopoverContent>
    );
  }

  // The one-item variant's button isn't focusable, so a click leaves focus on the root.
  _onRootKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.target === this._root) this._forwardKeyToMenu(event);
  };

  /* While the menu is open, focus stays on the button that opened it, so the arrows and Enter
     pressed there are handed to the menu, which has its own keyboard handling (Menu); Enter picks
     like a click does. They stop here, so they don't also reach the global keymap
     (core:next-item and the like). */
  _forwardKeyToMenu(event: React.KeyboardEvent<HTMLDivElement>) {
    if (this.state.open === false || !MENU_KEYS.includes(event.key) || !this._root) return false;
    const content = this._root.querySelector('[data-slot="popover-content"]');
    if (!content || content.contains(event.target as Node)) return false;
    const target = content.querySelector('[data-slot="menu"]') || content.firstElementChild;
    if (!target) return false;
    event.preventDefault();
    event.stopPropagation();
    target.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: event.key,
        code: event.nativeEvent.code,
        shiftKey: event.shiftKey,
        bubbles: true,
        cancelable: true,
      })
    );
    if (event.key === 'Enter' && this.props.closeOnMenuClick) this.setState({ open: false });
    return true;
  }

  _onOpenChange = (open: boolean) => {
    if (!open) this.setState({ open: false });
  };

  toggleDropdown = () => {
    if (this.state.open !== false) {
      this.setState({ open: false });
    } else {
      const buttonBottom = (ReactDOM.findDOMNode(this) as HTMLElement).getBoundingClientRect()
        .bottom;
      if (buttonBottom + 200 > window.innerHeight) {
        this.setState({ open: 'up' });
      } else {
        this.setState({ open: 'down' });
      }
    }
  };

  _onMenuClick = (event: React.MouseEvent<HTMLDivElement>) => {
    if (this.props.closeOnMenuClick) {
      this.setState({ open: false });
    }
  };

  _onBlur = (event: React.FocusEvent<HTMLDivElement>) => {
    const target = event.nativeEvent.relatedTarget as Node | null;
    if (target != null && this._root && this._root.contains(target)) {
      return;
    }
    this.setState({ open: false });
  };
}
