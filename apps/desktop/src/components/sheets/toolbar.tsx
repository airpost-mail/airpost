/* eslint react/prefer-stateless-function: 0 */
/* eslint global-require: 0 */
import React from 'react';
import ReactDOM from 'react-dom';

import { localized, isRTL } from '../../intl/localize';
import * as Actions from '../../data/actions';
import ComponentRegistry from '../../packages/component-registry';
import WorkspaceStore from '../../data/stores/workspace';
import { SheetDeclaration } from '../../data/stores/workspace';
import { Flexbox } from '../flexbox';
import { RetinaImg } from '../retina-img';
import { RovingTabIndexToolbar } from '../roving-tab-index-toolbar';
import * as Utils from '../../data/models/utils';
import { Disposable } from 'rx-core';
import { isWaylandSession } from '../../platform/linux-desktop';
import { SheetDepthContext } from './context';
import { Button } from '../kit/button';
import { IconButton } from '../kit/icon-button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../kit/tooltip';

let Category = null;
let FocusedPerspectiveStore = null;

class ToolbarSpacer extends React.Component<{ order: number }> {
  static displayName = 'ToolbarSpacer';

  render() {
    return <div className="item-spacer" style={{ flex: 1, order: this.props.order || 0 }} />;
  }
}

class WindowTitle extends React.Component<Record<string, unknown>, { title: string }> {
  static displayName = 'WindowTitle';

  private disposable?: Disposable;

  constructor(props) {
    super(props);
    this.state = AppEnv.getLoadSettings();
  }

  componentDidMount() {
    this.disposable = AppEnv.onWindowPropsReceived(() => this.setState(AppEnv.getLoadSettings()));
  }

  componentWillUnmount() {
    if (this.disposable) {
      this.disposable.dispose();
    }
  }

  render() {
    return <div className="window-title">{this.state.title}</div>;
  }
}

class ToolbarBack extends React.Component<Record<string, unknown>, { categoryName: string }> {
  static displayName = 'ToolbarBack';

  // These stores are only required when this Toolbar is actually needed.
  // This is because loading these stores has database side effects.
  constructor(props) {
    super(props);
    Category = Category || require('../../data/models/category').Category;
    FocusedPerspectiveStore =
      FocusedPerspectiveStore || require('../../data/stores/focused-perspective').default;

    this.state = {
      categoryName: FocusedPerspectiveStore.current().name,
    };
  }

  _unsubscriber?: () => void;

  componentDidMount() {
    this._unsubscriber = FocusedPerspectiveStore.listen(() =>
      this.setState({ categoryName: FocusedPerspectiveStore.current().name })
    );
  }

  componentWillUnmount() {
    if (this._unsubscriber) {
      this._unsubscriber();
    }
  }

  _onClick = () => {
    Actions.popSheet();
  };

  render() {
    let title = localized('Back');
    if (this.state.categoryName === Category.AllMailName) {
      title = localized('All Mail');
    } else if (this.state.categoryName === 'INBOX') {
      title = localized('Inbox');
    } else {
      title = this.state.categoryName;
    }
    const label = localized(`Return to %@`, title);
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          {/* The trigger's own data-slot would replace the button's. */}
          <Button
            data-slot="button"
            variant="ghost"
            className="item-back"
            aria-label={label}
            onClick={this._onClick}
          >
            <RetinaImg
              name="sheet-back.png"
              mode={RetinaImg.Mode.ContentIsMask}
              style={isRTL ? { transform: `scaleX(-1)` } : {}}
            />
            <div className="item-back-title">{title}</div>
          </Button>
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
    );
  }
}

class ToolbarWindowControls extends React.Component<Record<string, unknown>, { alt: boolean }> {
  static displayName = 'ToolbarWindowControls';

  constructor(props) {
    super(props);
    this.state = { alt: AppEnv.keymaps.getIsAltKeyDown() };
  }

  componentDidMount() {
    if (process.platform === 'darwin') {
      document.addEventListener(AppEnv.keymaps.EVENT_ALT_KEY_STATE_CHANGE, this._onAlt);
    }
  }

  componentWillUnmount() {
    if (process.platform === 'darwin') {
      document.removeEventListener(AppEnv.keymaps.EVENT_ALT_KEY_STATE_CHANGE, this._onAlt);
    }
  }

  _onAlt = () => {
    this.setState({ alt: AppEnv.keymaps.getIsAltKeyDown() });
  };

  _onMaximize = (event: React.MouseEvent<HTMLButtonElement>) => {
    if (process.platform === 'darwin' && !event.altKey) {
      AppEnv.setFullScreen(!AppEnv.isFullScreen());
    } else {
      AppEnv.maximize();
    }
  };

  render() {
    const enabled =
      process.platform === 'darwin' ||
      (process.platform === 'linux' &&
        AppEnv.config.get('core.workspace.menubarStyle') === 'hamburger');

    if (!enabled) {
      return <span />;
    }

    return (
      <RovingTabIndexToolbar
        label={localized('Window Controls')}
        className={`toolbar-window-controls alt-${this.state.alt}`}
      >
        {/* eslint-disable raw-controls -- the window's traffic lights are drawn by their own CSS */}
        <button
          tabIndex={-1}
          className="close"
          aria-label={localized('Close window')}
          onClick={() => AppEnv.close()}
        />
        <button
          tabIndex={-1}
          className="minimize"
          aria-label={localized('Minimize window')}
          onClick={() => AppEnv.minimize()}
        />
        <button
          tabIndex={-1}
          className="maximize"
          aria-label={localized('Maximize window')}
          onClick={this._onMaximize}
        />
        {/* eslint-enable raw-controls */}
      </RovingTabIndexToolbar>
    );
  }
}

class ToolbarMenuControl extends React.Component {
  static displayName = 'ToolbarMenuControl';

  _onOpenMenu = () => {
    const { applicationMenu } = require('@electron/remote').getGlobal('application');
    applicationMenu.menu.popup({});
  };

  render() {
    // On Wayland, the native menu bar does not render (Electron's ozone-wayland
    // backend bypasses GTK, and KDE Plasma's Global Menu may not be configured).
    // Show the hamburger menu button as a fallback so menus are always accessible.
    const enabled =
      process.platform === 'win32' ||
      (process.platform === 'linux' &&
        (AppEnv.config.get('core.workspace.menubarStyle') === 'hamburger' || isWaylandSession()));

    if (!enabled) {
      return <span />;
    }

    return (
      <div className="toolbar-menu-control">
        <IconButton
          variant="toolbar"
          label={localized('Application menu')}
          icon="windows-menu-icon.png"
          onClick={this._onOpenMenu}
        />
      </div>
    );
  }
}

type ToolbarItem = typeof React.Component & { displayName?: string };

// macOS and other platforms do not flip the corner that the window controls
// appear in when displaying the UI RTL, but Chrome flips our whole UI. We need
// to intentionally undo the flip.
const GLOBAL_START_ITEMS = [isRTL ? ToolbarMenuControl : ToolbarWindowControls] as ToolbarItem[];
const GLOBAL_END_ITEMS = [isRTL ? ToolbarWindowControls : ToolbarMenuControl] as ToolbarItem[];

/* The items every sheet's toolbar starts and ends with: the window controls or menu button, then
   what plugins register at Sheet.Global.Toolbar.Left and .Right. */
function globalToolbarItems(side: 'start' | 'end', mode: string): ToolbarItem[] {
  const location =
    side === 'start'
      ? WorkspaceStore.Sheet.Global.Toolbar.Left
      : WorkspaceStore.Sheet.Global.Toolbar.Right;
  return [
    ...(side === 'start' ? GLOBAL_START_ITEMS : GLOBAL_END_ITEMS),
    ...ComponentRegistry.findComponentsMatching({ location, mode }),
  ];
}

/*
Double-clicking a title bar zooms the window, as macOS does for native title bars. A frameless
window doesn't get that from the drag region, so the toolbar and SheetTitleBar call this on click.
*/
let lastTitleBarClickTime = 0;

export function onTitleBarClick(e: React.MouseEvent<HTMLElement>) {
  if (process.platform !== 'darwin') return;
  if (e.target instanceof HTMLElement) {
    if (['INPUT', 'A', 'BUTTON'].includes(e.target.tagName)) return;
    if (e.target.hasAttribute('contenteditable')) return;
  }

  if (Date.now() - lastTitleBarClickTime < 350) {
    const win = AppEnv.getCurrentWindow();
    if (win.isMaximized()) {
      win.unmaximize();
    } else {
      win.maximize();
    }
  }
  lastTitleBarClickTime = Date.now();
}

function itemContainer(
  components: Array<typeof React.Component & { displayName?: string }>,
  props: { data: SheetDeclaration; depth: number }
) {
  return (
    <Flexbox className="item-container" direction="row">
      {components.map((Component) => (
        <Component key={Component.displayName} {...props} />
      ))}
      <ToolbarSpacer key="spacer-50" order={-50} />
      <ToolbarSpacer key="spacer+50" order={50} />
    </Flexbox>
  );
}

interface SheetTitleBarProps {
  /** `start` holds what the toolbar puts at its start: the window controls, the sheet's left items
   * and the back button. `end` holds the sheet's right items and the menu button. */
  side: 'start' | 'end';
}

/*
The top strip of a sheet declared with `hideToolbar`, which runs to the top of the window in place
of the shared toolbar. The sheet puts `side="start"` at the top of its first column and
`side="end"` at the top of its last; both drag the window.
*/
export function SheetTitleBar({ side }: SheetTitleBarProps) {
  const depth = React.useContext(SheetDepthContext);
  const [, rerender] = React.useReducer((n: number) => n + 1, 0);
  React.useEffect(() => {
    const unlisten = ComponentRegistry.listen(() => rerender());
    return () => {
      unlisten();
    };
  }, []);

  // Undefined while the sheet animates out after being popped.
  const sheet = WorkspaceStore.sheetStack()[depth];
  const mode = WorkspaceStore.layoutMode();
  const find = (location: { id: string }) =>
    ComponentRegistry.findComponentsMatching({ location, mode });

  const components =
    side === 'start'
      ? [
          ...globalToolbarItems('start', mode),
          ...(sheet ? find(sheet.Toolbar.Left) : []),
          ...(depth > 0 ? [ToolbarBack] : []),
        ]
      : [...(sheet ? find(sheet.Toolbar.Right) : []), ...globalToolbarItems('end', mode)];

  return (
    <div
      role="toolbar"
      aria-label={localized('Toolbar')}
      className={`sheet-title-bar sheet-title-bar-${side}`}
      onClick={onTitleBarClick}
    >
      {itemContainer(components, { data: sheet, depth })}
    </div>
  );
}

interface ToolbarProps {
  data: SheetDeclaration;
  depth: number;
}

interface ToolbarState {
  mode: string;
  columns: Array<Array<typeof React.Component>>;
  columnNames: string[];
}

const COLUMN_ARIA_LABELS: Record<string, string> = {
  RootSidebar: localized('Sidebar toolbar'),
  ThreadList: localized('Thread list toolbar'),
  MessageList: localized('Message toolbar'),
  MessageListSidebar: localized('Contact panel toolbar'),
  CalendarSidebar: localized('Sidebar toolbar'),
  CalendarContent: localized('Calendar toolbar'),
};

let lastReportedToolbarHeight = 0;

export default class Toolbar extends React.Component<ToolbarProps, ToolbarState> {
  static displayName = 'Toolbar';

  mounted = false;
  unlisteners: Array<() => void> = [];

  constructor(props) {
    super(props);
    this.state = this._getStateFromStores();
  }

  componentDidMount() {
    this.mounted = true;
    this.unlisteners = [];
    this.unlisteners.push(WorkspaceStore.listen(() => this.setState(this._getStateFromStores())));
    this.unlisteners.push(
      ComponentRegistry.listen(() => this.setState(this._getStateFromStores()))
    );
    window.addEventListener('resize', this._onWindowResize);
    window.requestAnimationFrame(() => this.recomputeLayout());
  }

  shouldComponentUpdate(nextProps: ToolbarProps, nextState: ToolbarState) {
    // This is very important. Because toolbar uses TransitionGroup,
    // repetitive unnecessary updates can break animations and cause performance issues.
    return !Utils.isEqualReact(nextProps, this.props) || !Utils.isEqualReact(nextState, this.state);
  }

  componentDidUpdate(prevProps: ToolbarProps) {
    if (prevProps.data !== this.props.data || prevProps.depth !== this.props.depth) {
      this.setState(this._getStateFromStores());
    }
    // Wait for other components that are dirty (the actual columns in the sheet)
    window.requestAnimationFrame(() => this.recomputeLayout());
  }

  componentWillUnmount() {
    this.mounted = false;
    window.removeEventListener('resize', this._onWindowResize);
    for (const u of this.unlisteners) {
      u();
    }
  }

  recomputeLayout() {
    // Yes this really happens - do not remove!
    if (!this.mounted) {
      return;
    }

    // Find our item containers that are tied to specific columns
    const el = ReactDOM.findDOMNode(this) as HTMLElement;
    const columnToolbarEls = el.querySelectorAll('[data-column]');

    // Find the top sheet in the stack
    const sheet = document.querySelectorAll("[data-role='Sheet']")[this.props.depth];
    if (!sheet) {
      return;
    }

    // The main window's nav rail sits beside the sheets, so columns start that far in. Measured
    // from <main> rather than the sheet, which slides in from the side when pushed.
    const elRect = el.getBoundingClientRect();
    const mainRect = sheet.closest('main').getBoundingClientRect();
    const inset = mainRect.left - elRect.left;

    // Position item containers so they have the position and width
    // as their respective columns in the top sheet
    for (const columnToolbarEl of Array.from(columnToolbarEls) as HTMLElement[]) {
      const column = columnToolbarEl.dataset.column;
      const columnEl = sheet.querySelector(`[data-column='${column}']`) as HTMLElement;
      if (!columnEl) {
        continue;
      }

      let left = inset + columnEl.offsetLeft;
      let width = columnEl.offsetWidth;
      // The first column holds the window controls, so it also covers the strip above the rail
      // to keep them in the corner. In RTL the controls are in the last column instead.
      if (column === '0' && !isRTL) {
        width += left;
        left = 0;
      }
      columnToolbarEl.style.display = 'inherit';
      columnToolbarEl.style.left = `${left}px`;
      columnToolbarEl.style.width = `${width}px`;
    }

    // Record our overall height for sheets. Skipped while a `hideToolbar` sheet hides the
    // toolbar: its title bar has the toolbar's height, so the last offset still fits.
    if (el.clientHeight > 0 && el.clientHeight !== lastReportedToolbarHeight) {
      lastReportedToolbarHeight = el.clientHeight;
      require('@electron/remote').getCurrentWindow().setSheetOffset(el.clientHeight);
    }
  }

  _onWindowResize = () => {
    this.recomputeLayout();
  };

  _getStateFromStores(props: ToolbarProps = this.props) {
    const state: ToolbarState = {
      mode: WorkspaceStore.layoutMode(),
      columns: [],
      columnNames: [],
    };

    // Add items registered to Regions in the current sheet
    if (props.data && props.data.columns[state.mode]) {
      for (const loc of props.data.columns[state.mode]) {
        if (WorkspaceStore.isLocationHidden(loc)) {
          continue;
        }
        const entries = ComponentRegistry.findComponentsMatching({
          location: loc.Toolbar,
          mode: state.mode,
        });
        state.columns.push(entries);
        if (entries) {
          state.columnNames.push(loc.Toolbar.id.split(':')[0]);
        }
      }
    }

    // Add left items registered to the Sheet instead of to a Region
    if (state.columns.length > 0) {
      state.columns[0].push(...globalToolbarItems('start', state.mode));
      state.columns[0].push(
        ...ComponentRegistry.findComponentsMatching({
          location: props.data.Toolbar.Left,
          mode: state.mode,
        })
      );
      if (props.depth > 0) {
        state.columns[0].push(ToolbarBack);
      }

      // Add right items registered to the Sheet instead of to a Region
      const last = state.columns[state.columns.length - 1];
      last.push(...globalToolbarItems('end', state.mode));
      last.push(
        ...ComponentRegistry.findComponentsMatching({
          location: props.data.Toolbar.Right,
          mode: state.mode,
        })
      );

      if (state.mode === 'popout') {
        state.columns[0].push(WindowTitle);
      }
    }

    return state;
  }

  render() {
    const toolbars = this.state.columns.map((components, idx) => (
      <div
        role="toolbar"
        aria-label={COLUMN_ARIA_LABELS[this.state.columnNames[idx]] || localized('Toolbar')}
        style={{ position: 'absolute', top: 0, display: 'none' }}
        className={`toolbar-${this.state.columnNames[idx]}`}
        data-column={idx}
        key={idx}
      >
        {itemContainer(components, this.props)}
      </div>
    ));

    return (
      <SheetDepthContext.Provider value={this.props.depth}>
        <div
          style={{
            position: 'absolute',
            width: '100%',
            height: '100%',
            zIndex: 1,
          }}
          className={`sheet-toolbar-container mode-${this.state.mode}`}
          data-id={this.props.data.id}
        >
          {toolbars}
        </div>
      </SheetDepthContext.Provider>
    );
  }
}
