import React from 'react';
import ReactDOM from 'react-dom';
import { RetinaImg } from './retina-img';
import { Menu, MenuProps } from './menu';
import { Popover, PopoverAnchor, PopoverContent } from './kit/popover';

const Attachment = {
  LeftEdge: 'LeftEdge',
  RightEdge: 'RightEdge',
};

interface DropdownMenuProps extends MenuProps {
  className?: string;
  intitialSelectionItem?: any;
  attachment?: string;
  headerComponents?: React.ReactNode;
}
type DropdownMenuState = {
  expanded: boolean;
  currentSelection: any;
};

/* Public: A button showing the current choice that opens a {Menu} of `items` below it. Takes
   the {Menu} props. The menu is the UI kit's Popover: it closes on a pick, a click outside,
   Escape and when focus leaves. It is rendered inside the component, so styles that reach the
   menu through `.dropdown-menu` keep applying. */
export default class DropdownMenu extends React.Component<DropdownMenuProps, DropdownMenuState> {
  static Attachment = Attachment;

  _root: HTMLDivElement | null = null;

  constructor(props: DropdownMenuProps) {
    super(props);
    this.state = {
      expanded: false,
      currentSelection: this.props.intitialSelectionItem,
    };
  }

  componentDidUpdate(prevProps: DropdownMenuProps) {
    if (prevProps.intitialSelectionItem !== this.props.intitialSelectionItem) {
      this.setState({ currentSelection: this.props.intitialSelectionItem });
    }
  }

  _toggleExpanded = () => {
    this.setState({ expanded: !this.state.expanded });
  };

  // As before the kit: the menu's search field takes focus if it has one; otherwise focus stays.
  _onOpenAutoFocus = (event: Event) => {
    event.preventDefault();
    const searchInput = (event.currentTarget as HTMLElement).querySelector('input');
    if (searchInput) {
      searchInput.focus();
    }
  };

  _close = () => {
    if (this.state.expanded) {
      this.setState({ expanded: false });
    }
  };

  _onSelect = (item: unknown) => {
    this.setState({ currentSelection: item });
    if (this.props.onSelect) {
      this.props.onSelect(item);
    }
    this._close();
  };

  _onBlur = (e: React.FocusEvent<HTMLDivElement>) => {
    const node = ReactDOM.findDOMNode(this);
    let otherNode = e.relatedTarget as Element | null;
    if (otherNode) {
      while (otherNode.parentElement) {
        // Don't close the dropdown if the related target is a child of this component
        if (otherNode.parentElement === node) {
          return;
        }
        otherNode = otherNode.parentElement;
      }
    }
    this._close();
  };

  render() {
    return (
      <Popover open={this.state.expanded} onOpenChange={(open) => !open && this._close()}>
        <PopoverAnchor asChild>
          <div
            ref={(el) => (this._root = el)}
            className={`btn dropdown-menu ${this.props.className}`}
            tabIndex={-1}
            onBlur={this._onBlur}
            style={{ display: 'inline-block', position: 'relative' }}
          >
            <div
              onClick={this._toggleExpanded}
              style={{
                cursor: 'pointer',
                display: 'flex',
                alignItems: 'center',
              }}
            >
              {this.props.itemContent(this.state.currentSelection)}
              <RetinaImg
                style={{ marginLeft: 5 }}
                name="dropdown-chevron.png"
                mode={RetinaImg.Mode.ContentDark}
              />
            </div>
          </div>
        </PopoverAnchor>
        {this.state.expanded && this._root ? (
          <PopoverContent
            data-variant="plain"
            // Portalled into the component itself rather than the window's portal root.
            container={this._root}
            side="bottom"
            align={this.props.attachment === Attachment.RightEdge ? 'end' : 'start'}
            sideOffset={0}
            // As before the kit, above the content around the button.
            style={{ zIndex: 10 }}
            collisionPadding={4}
            onOpenAutoFocus={this._onOpenAutoFocus}
            onCloseAutoFocus={(e) => e.preventDefault()}
            onInteractOutside={(e) => {
              // The button toggles the menu itself.
              if (this._root && this._root.contains(e.target as Node)) e.preventDefault();
            }}
          >
            <Menu {...this.props} onEscape={this._close} onSelect={this._onSelect} />
          </PopoverContent>
        ) : null}
      </Popover>
    );
  }
}
