import React from 'react';
import { CSSTransition, TransitionGroup } from 'react-transition-group';
import { localized } from '../../intl/localize';
import WorkspaceStore from '../../data/stores/workspace';

import Sheet from './sheet';
import Toolbar, { onTitleBarClick } from './toolbar';
import { Flexbox } from '../flexbox';
import { InjectedComponentSet } from '../injected/component-set';
import {
  NavRail,
  NavRailRestoreButton,
  SHOW_NAV_RAIL_CONFIG_KEY,
  isNavRailShown,
  registerNavRailCommands,
} from '../nav-rail';
import { SheetDeclaration } from '../../data/stores/workspace';
import { Disposable } from 'rx-core';

interface SheetContainerState {
  stack: SheetDeclaration[];
  mode: string;
  showNavRail: boolean;
  error?: string;
}

/*
Marks a stacked-under subtree inert, so it takes no focus and no pointer events.

Spread rather than written as `inert={...}`: `inert` is a real HTML attribute that React
passes straight through, but it is absent from React 17's JSX types. The augmentation in
window/react.d.ts declares it and does not take effect, because two copies of
@types/react are installed - one under app/node_modules, one at the repo root - and both
register a global JSX namespace, so the copy that wins the JSX check is not the copy the
`declare module 'react'` augmentation merges into. Spreading sidesteps the attribute-name
check without asserting anything about the element.
*/
function inertWhenStacked(stacked: boolean): { inert?: '' } {
  return stacked ? { inert: '' } : {};
}

export default class SheetContainer extends React.Component<
  Record<string, unknown>,
  SheetContainerState
> {
  static displayName = 'SheetContainer';

  // Only recomputeLayout is called on these.
  _toolbarComponents: { [depth: number]: Pick<Toolbar, 'recomputeLayout'> | null } = {};
  _lastChromeKey?: string;
  unsubscribe?: () => void;
  windowPropsDisposable?: Disposable;
  navRailConfigDisposable?: Disposable;
  navRailCommandsDisposable?: Disposable;

  constructor(props: Record<string, unknown>) {
    super(props);
    this.state = this._getStateFromStores();
  }

  componentDidMount() {
    this._lastChromeKey = this._chromeKey();
    this.unsubscribe = WorkspaceStore.listen(this._onStoreChange);
    // A hot window mounts with the empty window's load settings and receives the real
    // ones (including `toolbar`) only when it is assigned a window type.
    this.windowPropsDisposable = AppEnv.onWindowPropsReceived(this._onStoreChange);

    if (AppEnv.isMainWindow()) {
      this.navRailConfigDisposable = AppEnv.config.observe(SHOW_NAV_RAIL_CONFIG_KEY, () => {
        this.navRailCommandsDisposable?.dispose();
        this.navRailCommandsDisposable = registerNavRailCommands(isNavRailShown());
        this._onStoreChange();
      });
    }
  }

  componentDidUpdate() {
    // The toolbars place their items over the sheet's columns, measured on resize. A resize while
    // Preferences hides the toolbar and the nav rail measures the columns without the rail, so
    // remeasure when either comes back (or goes) rather than wait for the next resize. Compared
    // with the last key rather than prevState: WorkspaceStore mutates the stack array in place.
    const key = this._chromeKey();
    if (key === this._lastChromeKey) return;
    this._lastChromeKey = key;
    window.requestAnimationFrame(() => {
      for (const toolbar of Object.values(this._toolbarComponents)) {
        toolbar?.recomputeLayout();
      }
    });
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    // We don't currently display the error, but we need to call setState within
    // this function or the component does not re-render after being reset.
    this.setState({ error: error.stack });
    AppEnv.reportError(error);
  }

  componentWillUnmount() {
    if (this.unsubscribe) {
      this.unsubscribe();
    }
    if (this.windowPropsDisposable) {
      this.windowPropsDisposable.dispose();
    }
    this.navRailConfigDisposable?.dispose();
    this.navRailCommandsDisposable?.dispose();
  }

  _getStateFromStores() {
    return {
      stack: WorkspaceStore.sheetStack(),
      mode: WorkspaceStore.layoutMode(),
      showNavRail: isNavRailShown(),
    };
  }

  _navRail() {
    const { stack, showNavRail } = this.state;
    const topSheet = stack[stack.length - 1];
    // Preferences takes over the window, so it shows neither the rail nor its restore button.
    const allowed = AppEnv.isMainWindow() && topSheet !== WorkspaceStore.Sheet.Preferences;
    return { allowed, shown: allowed && showNavRail };
  }

  /** What around the sheets moves their columns: the rail and whether the toolbar shows. */
  _chromeKey() {
    const { stack } = this.state;
    return `${this._navRail().shown}:${!!stack[stack.length - 1]?.hideToolbar}`;
  }

  _onColumnSizeChanged = (sheet: Sheet) => {
    const toolbar = this._toolbarComponents[sheet.props.depth];
    if (toolbar) {
      toolbar.recomputeLayout();
    }
    window.dispatchEvent(new Event('resize'));
  };

  _onStoreChange = () => {
    this.setState(this._getStateFromStores());
  };

  _toolbarContainerElement() {
    const { toolbar } = AppEnv.getLoadSettings();
    if (!toolbar) {
      return [];
    }

    const components = this.state.stack.map((sheet, index) => (
      <Toolbar
        data={sheet}
        ref={(cm) => {
          this._toolbarComponents[index] = cm;
        }}
        key={`${index}:${sheet.id}:toolbar`}
        depth={index}
      />
    ));

    // A sheet that hides the toolbar draws a SheetTitleBar instead. The toolbar stays mounted
    // under it so the sheets below keep their toolbar items' state (e.g. a search in progress).
    const topSheet = this.state.stack[this.state.stack.length - 1];
    const hidden = !!topSheet?.hideToolbar;

    return (
      <header
        className="sheet-toolbar"
        role="banner"
        aria-label={localized('Application toolbar')}
        style={{ order: 0, zIndex: 3, ...(hidden ? { display: 'none' } : {}) }}
        onClick={onTitleBarClick}
      >
        <div {...inertWhenStacked(this.state.stack.length > 1)}>{components[0]}</div>
        <TransitionGroup component={null}>
          {components.slice(1).map((comp) => (
            <CSSTransition key={comp.key} classNames="opacity-125ms" timeout={125}>
              {comp}
            </CSSTransition>
          ))}
        </TransitionGroup>
      </header>
    );
  }

  render() {
    const totalSheets = this.state.stack.length;
    const topSheet = this.state.stack[totalSheets - 1];
    const { allowed: navRailAllowed, shown: hasNavRail } = this._navRail();

    if (!topSheet) {
      return <div />;
    }

    const sheetComponents = this.state.stack.map((sheet, index) => (
      <Sheet
        data={sheet}
        depth={index}
        key={index > 0 ? `${index}:${sheet.id}` : `root`}
        onColumnSizeChanged={this._onColumnSizeChanged}
      />
    ));

    return (
      <Flexbox
        direction="column"
        className={`layout-mode-${this.state.mode}`}
        style={{ overflow: 'hidden' }}
      >
        {this._toolbarContainerElement()}

        <Flexbox
          direction="row"
          className={hasNavRail ? 'nav-rail-frame' : undefined}
          style={{ order: 1, flex: 1, minHeight: 0 }}
        >
          {hasNavRail && <NavRail />}
          <Flexbox
            direction="column"
            className={hasNavRail ? 'nav-rail-content' : undefined}
            // The inset card's margin and border would overflow a 100% height.
            height={hasNavRail ? 'auto' : undefined}
            style={{ flex: 1, minWidth: 0 }}
          >
            {navRailAllowed && !hasNavRail && <NavRailRestoreButton />}
            <div style={{ order: 1, zIndex: 2 }}>
              <InjectedComponentSet
                matching={{ locations: [topSheet.Header, WorkspaceStore.Sheet.Global.Header] }}
                direction="column"
                id={topSheet.id}
              />
            </div>

            <main
              style={{ order: 2, flex: 1, position: 'relative', zIndex: 1 }}
              aria-label={localized('Email workspace')}
            >
              <div {...inertWhenStacked(totalSheets > 1)}>{sheetComponents[0]}</div>
              <TransitionGroup component={null}>
                {sheetComponents.slice(1).map((comp) => (
                  <CSSTransition key={comp.key} classNames="sheet-stack" timeout={125}>
                    {comp}
                  </CSSTransition>
                ))}
              </TransitionGroup>
            </main>

            <footer style={{ order: 3, zIndex: 4 }}>
              <InjectedComponentSet
                matching={{ locations: [topSheet.Footer, WorkspaceStore.Sheet.Global.Footer] }}
                direction="column"
                id={topSheet.id}
              />
            </footer>
          </Flexbox>
        </Flexbox>
      </Flexbox>
    );
  }
}
