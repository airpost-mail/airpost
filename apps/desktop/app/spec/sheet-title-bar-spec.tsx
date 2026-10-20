import React from 'react';
import ReactDOM from 'react-dom';
import {
  Actions,
  ComponentRegistry,
  FocusedPerspectiveStore,
  WorkspaceStore,
} from 'mailspring-exports';

import SheetContainer from '../src/sheet-container';
import Sheet from '../src/sheet';
import { SheetTitleBar } from '../src/sheet-toolbar';
import { SheetDepthContext } from '../src/sheet-context';

const fakeSheet = (id: string, extra = {}) =>
  ({
    id,
    columns: {},
    Toolbar: { Left: { id: `${id}:Toolbar:Left` }, Right: { id: `${id}:Toolbar:Right` } },
    Header: { id: `${id}:Header` },
    Footer: { id: `${id}:Footer` },
    ...extra,
  }) as any;

// Stand-ins for the window controls and menu button, which register at these locations.
const GlobalLeft = () => <div className="global-left" />;
GlobalLeft.displayName = 'GlobalLeft';
const GlobalRight = () => <div className="global-right" />;
GlobalRight.displayName = 'GlobalRight';

describe('SheetTitleBar', function sheetTitleBarSpec() {
  let container: HTMLDivElement;
  let stack: any[];
  const threads = fakeSheet('Threads');
  const preferences = fakeSheet('Preferences', { hideToolbar: true });

  beforeEach(() => {
    spyOn(WorkspaceStore, 'sheetStack').andCallFake(() => stack);
    spyOn(WorkspaceStore, 'topSheet').andCallFake(() => stack[stack.length - 1]);
    spyOn(WorkspaceStore, 'layoutMode').andReturn('list');
    spyOn(FocusedPerspectiveStore, 'current').andReturn({ name: 'INBOX' } as any);
    stack = [threads, preferences];
    ComponentRegistry.register(GlobalLeft, { location: WorkspaceStore.Sheet.Global.Toolbar.Left });
    ComponentRegistry.register(GlobalRight, {
      location: WorkspaceStore.Sheet.Global.Toolbar.Right,
    });
    container = document.createElement('div');
    document.body.appendChild(container);
  });

  afterEach(() => {
    ReactDOM.unmountComponentAtNode(container);
    container.remove();
    ComponentRegistry.unregister(GlobalLeft);
    ComponentRegistry.unregister(GlobalRight);
  });

  const renderBar = (side: 'start' | 'end', depth: number) =>
    ReactDOM.render(
      <SheetDepthContext.Provider value={depth}>
        <SheetTitleBar side={side} />
      </SheetDepthContext.Provider>,
      container
    );

  it('puts the window controls and the back button at the start of a pushed sheet', () => {
    renderBar('start', 1);
    const bar = container.querySelector('.sheet-title-bar-start');
    expect(bar.getAttribute('role')).toBe('toolbar');
    expect(bar.querySelector('.global-left')).not.toBe(null);
    expect(bar.querySelector('.global-right')).toBe(null);
    expect(bar.querySelector('.item-back-title').textContent).toBe('Inbox');

    spyOn(Actions, 'popSheet');
    (bar.querySelector('.item-back') as HTMLElement).click();
    expect(Actions.popSheet).toHaveBeenCalled();
  });

  it('has no back button on the root sheet', () => {
    renderBar('start', 0);
    expect(container.querySelector('.item-back')).toBe(null);
  });

  it('puts the menu button, and not the back button, at the end', () => {
    renderBar('end', 1);
    const bar = container.querySelector('.sheet-title-bar-end');
    expect(bar.querySelector('.global-right')).not.toBe(null);
    expect(bar.querySelector('.global-left')).toBe(null);
    expect(bar.querySelector('.item-back')).toBe(null);
  });

  describe('in the sheet container', () => {
    beforeEach(() => {
      spyOn(AppEnv, 'isMainWindow').andReturn(false);
      const loadSettings = AppEnv.getLoadSettings();
      spyOn(AppEnv, 'getLoadSettings').andReturn({ ...loadSettings, toolbar: true });
      spyOn(AppEnv, 'setMinimumWidth');
      spyOn(Sheet.prototype as any, '_buildState').andReturn({ columns: [], mode: 'list' });
      spyOn(Sheet.prototype, 'render').andReturn(null);
    });

    it('hides the toolbar while a hideToolbar sheet is on top, keeping it mounted', () => {
      stack = [threads];
      ReactDOM.render(<SheetContainer />, container);
      const header = () => container.querySelector('header.sheet-toolbar') as HTMLElement;
      expect(header().style.display).toBe('');

      stack = [threads, preferences];
      WorkspaceStore.trigger();
      expect(header().style.display).toBe('none');
      expect(header().querySelector('[data-id="Threads"]')).not.toBe(null);

      stack = [threads];
      WorkspaceStore.trigger();
      expect(header().style.display).toBe('');
    });
  });
});
