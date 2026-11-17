// SPDX-License-Identifier: GPL-3.0-only
// The registry slots the events, list-unsubscribe, thread-search and onboarding features keep
// registering into, because plugins (or the documented third-party usage) use them too. Each spec
// renders the slot's consumer with the feature's component and a fake one registered side by side.
import React from 'react';
import { render, cleanup } from '@testing-library/react';
import ComponentRegistry from '../../src/packages/component-registry';
import { Message } from '../../src/data/models/message';
import WorkspaceStore from '../../src/data/stores/workspace';
import MessageItemBody from '../../src/features/message-list/message-item-body';
import Toolbar from '../../src/components/sheets/toolbar';
import Sheet from '../../src/components/sheets/sheet';
import * as Events from '../../src/features/events';
import * as Unsubscribe from '../../src/features/list-unsubscribe';
import * as ThreadSearch from '../../src/features/thread-search';
import * as Onboarding from '../../src/features/onboarding';

function fake(displayName: string) {
  const C = (props: any) => (
    <span className={`fake-${displayName}`} data-message={props.message && props.message.id} />
  );
  C.displayName = displayName;
  return C;
}

const names = (matching: object) =>
  ComponentRegistry.findComponentsMatching(matching).map((c) => c.displayName);

describe('Feature slots', function () {
  const registered: any[] = [];
  const register = (component: any, options: any) => {
    ComponentRegistry.register(component, options);
    registered.push(component);
  };

  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => {
    cleanup();
    registered.splice(0).forEach((c) => ComponentRegistry.unregister(c));
  });

  describe('message:BodyHeader', () => {
    // The translation plugin and third-party plugins render here too.
    beforeEach(() => {
      spyOn(AppEnv, 'isMainWindow').andReturn(false);
      Events.activate();
      Unsubscribe.activate();
    });

    afterEach(() => {
      Events.deactivate();
      Unsubscribe.deactivate();
    });

    it('holds the event and unsubscribe headers next to plugin headers', () => {
      register(fake('FakeBodyHeader'), { role: 'message:BodyHeader' });
      const registeredNames = names({ role: 'message:BodyHeader' });
      for (const name of ['EventHeaderContainer', 'UnsubscribeContainer', 'FakeBodyHeader']) {
        expect(registeredNames).toContain(name);
      }

      const message = new Message({
        id: 'msg-1',
        accountId: TEST_ACCOUNT_ID,
        body: 'Hello there',
        files: [],
        listUnsubscribe: '<https://example.com/unsub>',
      } as any);
      const { container } = render(<MessageItemBody message={message} downloads={{}} />);
      expect(container.querySelector('.fake-FakeBodyHeader').getAttribute('data-message')).toBe(
        'msg-1'
      );
      expect(container.querySelector('.unsubscribe-action').textContent).toBe('Unsubscribe');
      // No invitation attached, so the event header renders nothing.
      expect(container.querySelector('.event-wrapper')).toBe(null);
    });
  });

  /** mailspring-sort-inbox registers its sort menu in this slot. */
  describe('ThreadList.Toolbar', () => {
    const store = WorkspaceStore as any;
    let savedLocation: any;
    const threadList = { id: 'ThreadList', Toolbar: { id: 'ThreadList:Toolbar' } };
    const sheet = {
      id: 'Threads',
      columns: { list: [threadList] },
      Toolbar: { Left: { id: 'Threads:Toolbar:Left' }, Right: { id: 'Threads:Toolbar:Right' } },
    } as any;

    beforeEach(() => {
      // Spec windows define no sheets, so the column gets a stand-in location.
      savedLocation = store.Location;
      store.Location = { ...savedLocation, ThreadList: threadList };
      spyOn(WorkspaceStore, 'layoutMode').andReturn('list');
      spyOn(WorkspaceStore, 'isLocationHidden').andReturn(false);
      // The search bar's tutorial tip checks that its sheet is the top one.
      spyOn(WorkspaceStore, 'topSheet').andReturn(sheet);
      spyOn(AppEnv, 'isMainWindow').andReturn(true);
      ThreadSearch.activate();
    });

    afterEach(() => {
      ThreadSearch.deactivate();
      store.Location = savedLocation;
    });

    it('renders the search bar and plugin items in the thread list column', () => {
      register(fake('FakeSortMenu'), { location: threadList.Toolbar });
      const { container } = render(<Toolbar data={sheet} depth={0} />);
      const column = container.querySelector('.toolbar-ThreadList');
      expect(column.getAttribute('role')).toBe('toolbar');
      expect(column.querySelector('.thread-search-bar')).not.toBe(null);
      expect(column.querySelector('.fake-FakeSortMenu')).not.toBe(null);
    });
  });

  describe('the onboarding window', () => {
    const store = WorkspaceStore as any;
    let savedLocation: any;
    const center = { id: 'Center', Toolbar: { id: 'Center:Toolbar' } };

    beforeEach(() => {
      savedLocation = store.Location;
      store.Location = { ...savedLocation, Center: center };
      spyOn(WorkspaceStore, 'defineSheet');
      spyOn(AppEnv.themes, 'forceBaseTheme');
      // With an account, activate() leaves launch-at-login alone.
      const get = AppEnv.config.get.bind(AppEnv.config);
      spyOn(AppEnv.config, 'get').andCallFake((key: string) =>
        key === 'accounts' ? [{ id: 'a1' }] : get(key)
      );
    });

    afterEach(() => {
      store.Location = savedLocation;
    });

    it('fills the Center location of its one sheet with the onboarding pages', () => {
      Onboarding.activate();
      const root = ComponentRegistry.findComponentsMatching({ location: center });
      root.forEach((c) => ComponentRegistry.unregister(c));
      expect(WorkspaceStore.defineSheet).toHaveBeenCalledWith(
        'Main',
        { root: true },
        { list: ['Center'] }
      );
      expect(root.map((c) => c.displayName)).toEqual(['OnboardingRoot']);

      // The sheet renders whatever is registered there; a stand-in keeps the pages out.
      register(fake('FakeCenter'), { location: center });
      spyOn(WorkspaceStore, 'layoutMode').andReturn('list');
      const sheet = { id: 'Main', columns: { list: [center] } } as any;
      const { container } = render(<Sheet data={sheet} depth={0} />);
      expect(container.querySelector('.fake-FakeCenter')).not.toBe(null);
    });
  });
});
