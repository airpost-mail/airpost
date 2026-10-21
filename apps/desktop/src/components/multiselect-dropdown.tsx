import React, { Component } from 'react';
import classNames from 'classnames';
import { RetinaImg } from './retina-img';
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from './kit/dropdown-menu';

// Every prop has a default in defaultProps, so all are required here; JSX callers still see
// them as optional.
type MultiselectDropdownProps = {
  className: string;
  items: any[];
  itemChecked: (...args: any[]) => any;
  onToggleItem: (...args: any[]) => any;
  itemKey: (...args: any[]) => any;
  buttonText: string;
  itemContent: (...args: any[]) => any;
  attachment: string;
};

type MultiselectDropdownState = {
  open: boolean;
};

/*
Public: A button that opens a menu of items, any number of which can be checked. Items can be
strings or objects. Picking an item calls `onToggleItem` and leaves the menu open.

The menu is the UI kit's DropdownMenu with checkbox items: the arrow keys move through it,
Space and Enter toggle, Escape and a click outside close it. The button keeps the legacy
`button-dropdown btn-multiselect` and `only-item` classes, the menu `menu secondary-items`, the
items `item` plus `checked`.

@param {object} props - props for MultiselectDropdown
@param {string} props.className - css class applied to the component
@param {array} props.items - items to be rendered in the dropdown
@param {props.itemChecked} - props.itemChecked -- a function to determine if the item should be checked or not
@param {props.onToggleItem} - props.onToggleItem -- function called when an item is clicked
@param {props.itemKey} - props.itemKey -- function that indicates how to select the key for each item
@param {props.buttonText} - props.buttonText -- string to be rendered in the button
@param {props.itemContent} - props.itemContent -- function that renders an item
@param {string} props.attachment - 'left' (the default) lines the menu up with the button's start edge, anything else with its end edge
**/
class MultiselectDropdown extends Component<MultiselectDropdownProps, MultiselectDropdownState> {
  static displayName = 'MultiselectDropdown';

  static defaultProps = {
    className: '',
    items: [],
    itemChecked: () => false,
    onToggleItem: () => {},
    itemKey: () => {},
    buttonText: '',
    itemContent: () => {},
    attachment: 'left',
  };

  state = { open: false };

  _renderItem = (item: any) => {
    const checked = !!this.props.itemChecked(item);
    return (
      <DropdownMenuCheckboxItem
        key={this.props.itemKey(item)}
        className={classNames({ checked })}
        checked={checked}
        // Toggling leaves the menu open, so several items can be picked in a row.
        onSelect={(event) => event.preventDefault()}
        onCheckedChange={() => this.props.onToggleItem(item)}
      >
        {this.props.itemContent(item)}
      </DropdownMenuCheckboxItem>
    );
  };

  render() {
    const { items, attachment, className } = this.props;
    const { open } = this.state;
    return (
      <DropdownMenu open={open} onOpenChange={(next) => this.setState({ open: next })}>
        <div
          className={classNames(
            'button-dropdown',
            'btn-multiselect',
            'bordered',
            { 'open open-down': open },
            className
          )}
        >
          <DropdownMenuTrigger asChild>
            <button type="button" className="only-item">
              <span>{this.props.buttonText}</span>
              <RetinaImg
                name={'icon-thread-disclosure.png'}
                style={{ marginLeft: 12 }}
                mode={RetinaImg.Mode.ContentIsMask}
              />
            </button>
          </DropdownMenuTrigger>
        </div>
        <DropdownMenuContent
          className={classNames('secondary-items', attachment)}
          align={attachment === 'left' ? 'start' : 'end'}
          sideOffset={2}
        >
          {items.map(this._renderItem)}
        </DropdownMenuContent>
      </DropdownMenu>
    );
  }
}
export default MultiselectDropdown;
