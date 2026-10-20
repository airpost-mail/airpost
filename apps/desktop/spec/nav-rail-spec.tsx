import React from 'react';
import ReactDOM from 'react-dom';
import { ipcRenderer } from 'electron';
import { Simulate } from 'react-dom/test-utils';
import { ComponentRegistry, FocusedPerspectiveStore, WorkspaceStore } from 'mailspring-exports';

import SheetContainer from '../src/sheet-container';
import Sheet from '../src/sheet';
import Config from '../src/config';
import { MailNavRailItem, SHOW_NAV_RAIL_CONFIG_KEY } from '../src/components/nav-rail';

const fakeSheet = (id: string) =>
  ({ id, Header: { id: `${id}:Header` }, Footer: { id: `${id}:Footer` } }) as any;

describe('NavRail', function navRailSpec() {
  let container: HTMLDivElement;
  let stack: any[];
  const threads = fakeSheet('Threads');
  const preferences = fakeSheet('Preferences');
  const realPreferences = WorkspaceStore.Sheet.Preferences;

  const rail = () => container.querySelector('.nav-rail');
  const restoreButton = () => container.querySelector('.nav-rail-restore') as HTMLButtonElement;
  const railItem = (label: string) =>
    container.querySelector(`.nav-rail-item[title="${label}"]`) as HTMLButtonElement;
  const setStack = (sheets: any[]) => {
    stack = sheets;
    WorkspaceStore.trigger();
  };

  beforeEach(() => {
    spyOn(AppEnv, 'isMainWindow').andReturn(true);
    const loadSettings = AppEnv.getLoadSettings();
    spyOn(AppEnv, 'getLoadSettings').andReturn({ ...loadSettings, toolbar: false });
    spyOn(AppEnv, 'setMinimumWidth');
    spyOn(Sheet.prototype as any, '_buildState').andReturn({ columns: [], mode: 'list' });
    spyOn(Sheet.prototype, 'render').andReturn(null);
    spyOn(WorkspaceStore, 'sheetStack').andCallFake(() => stack);
    spyOn(WorkspaceStore, 'topSheet').andCallFake(() => stack[stack.length - 1]);
    spyOn(WorkspaceStore, 'layoutMode').andReturn('list');
    spyOn(ipcRenderer, 'send');
    WorkspaceStore.Sheet.Preferences = preferences;
    stack = [threads];

    ComponentRegistry.register(MailNavRailItem, { role: 'NavRail:Item' });
    container = document.createElement('div');
    document.body.appendChild(container);
    ReactDOM.render(<SheetContainer />, container);
  });

  afterEach(() => {
    ReactDOM.unmountComponentAtNode(container);
    container.remove();
    WorkspaceStore.Sheet.Preferences = realPreferences;
  });

  describe('show / hide', () => {
    it('shows the rail by default, with no restore button', () => {
      expect(AppEnv.config.get(SHOW_NAV_RAIL_CONFIG_KEY)).toBe(true);
      expect(rail()).not.toBe(null);
      expect(container.querySelector('.nav-rail-frame')).not.toBe(null);
      expect(restoreButton()).toBe(null);
    });

    it('hides from its own button, persists that, and comes back from the restore button', () => {
      Simulate.click(container.querySelector('.nav-rail-toggle'));

      expect(Config.prototype.setRawValue).toHaveBeenCalledWith(SHOW_NAV_RAIL_CONFIG_KEY, false);
      expect(AppEnv.config.get(SHOW_NAV_RAIL_CONFIG_KEY)).toBe(false);
      expect(rail()).toBe(null);
      expect(container.querySelector('.nav-rail-frame')).toBe(null);
      expect(container.querySelector('.nav-rail-content')).toBe(null);
      expect(restoreButton()).not.toBe(null);

      Simulate.click(restoreButton());

      expect(AppEnv.config.get(SHOW_NAV_RAIL_CONFIG_KEY)).toBe(true);
      expect(rail()).not.toBe(null);
      expect(restoreButton()).toBe(null);
    });

    it('registers only the menu command that applies, and both toggle the rail', () => {
      const count = (command: string) => AppEnv.commands.listenerCountForCommand(command);
      expect(count('navigation:hide-nav-rail')).toBe(1);
      expect(count('navigation:show-nav-rail')).toBe(0);

      AppEnv.commands.dispatch('navigation:hide-nav-rail');
      expect(AppEnv.config.get(SHOW_NAV_RAIL_CONFIG_KEY)).toBe(false);
      expect(rail()).toBe(null);
      expect(count('navigation:hide-nav-rail')).toBe(0);
      expect(count('navigation:show-nav-rail')).toBe(1);

      AppEnv.commands.dispatch('navigation:show-nav-rail');
      expect(AppEnv.config.get(SHOW_NAV_RAIL_CONFIG_KEY)).toBe(true);
      expect(rail()).not.toBe(null);
    });
  });

  describe('in Preferences', () => {
    it('shows neither the rail nor, when hidden, the restore button', () => {
      setStack([threads, preferences]);
      expect(rail()).toBe(null);
      expect(container.querySelector('.nav-rail-frame')).toBe(null);
      expect(restoreButton()).toBe(null);

      setStack([threads]);
      expect(rail()).not.toBe(null);

      AppEnv.config.set(SHOW_NAV_RAIL_CONFIG_KEY, false);
      setStack([threads, preferences]);
      expect(rail()).toBe(null);
      expect(restoreButton()).toBe(null);

      setStack([threads]);
      expect(restoreButton()).not.toBe(null);
    });
  });

  describe('items', () => {
    it('opens the Contacts window from a Contacts item that is never active', () => {
      const contacts = railItem('Contacts');
      expect(contacts).not.toBe(null);
      expect(contacts.classList.contains('active')).toBe(false);

      Simulate.click(contacts);
      expect(ipcRenderer.send).toHaveBeenCalledWith('command', 'application:show-contacts');
    });

    it('lights up Mail only while the mailbox is the root sheet', () => {
      let showingMail = true;
      spyOn(FocusedPerspectiveStore, 'isShowingMail').andCallFake(() => showingMail);
      WorkspaceStore.trigger();
      expect(railItem('Mail').classList.contains('active')).toBe(true);
      expect(railItem('Contacts').classList.contains('active')).toBe(false);

      showingMail = false;
      setStack([fakeSheet('Calendar')]);
      expect(railItem('Mail').classList.contains('active')).toBe(false);
      expect(railItem('Contacts').classList.contains('active')).toBe(false);
    });
  });
});
