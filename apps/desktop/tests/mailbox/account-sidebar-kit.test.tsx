// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup, act } from '@testing-library/react';
import { Account } from '../../src/data/models/account';
import { AccountStore } from '../../src/data/stores/account';
import CategoryStore from '../../src/data/stores/category';
import ComponentRegistry from '../../src/packages/component-registry';
import * as ExtensionRegistry from '../../src/packages/extension-registry';
import { Folder } from '../../src/data/models/folder';
import FocusedPerspectiveStore from '../../src/data/stores/focused-perspective';
import { MailboxPerspective } from '../../src/data/mailbox-perspective';
import WorkspaceStore from '../../src/data/stores/workspace';
import Sheet from '../../src/components/sheets/sheet';
import AccountSwitcher from '../../src/features/account-sidebar/account-switcher';
import AccountSidebar from '../../src/features/account-sidebar/account-sidebar';
import SidebarStore from '../../src/features/account-sidebar/sidebar-store';
import { Menus } from '../../src/platform/app-api';

describe('AccountSwitcher on the UI kit', function () {
  let popup: jasmine.Spy;

  beforeEach(() => {
    popup = spyOn(Menus, 'popup').andReturn(Promise.resolve());
  });

  afterEach(() => cleanup());

  const renderSwitcher = () =>
    render(
      <AccountSwitcher
        accounts={AccountStore.accounts()}
        sidebarAccountIds={[AccountStore.accounts()[0].id]}
      />
    );

  it('is an IconButton that keeps the account-switcher class', () => {
    const { container } = renderSwitcher();
    const button = container.querySelector('button.account-switcher');
    expect(button).not.toBeNull();
    expect(button.getAttribute('data-slot')).toBe('icon-button');
    expect(button.getAttribute('aria-label')).toBe('Account switcher');
    expect(button.getAttribute('aria-haspopup')).toBe('menu');
    expect(button.querySelector('img.content-mask')).not.toBeNull();
  });

  it('opens the native menu on mouse down', () => {
    const { container } = renderSwitcher();
    fireEvent.mouseDown(container.querySelector('.account-switcher'));
    expect(popup.callCount).toBe(1);
    const labels = popup.calls[0].args[0].map((i) => i.label);
    expect(labels).toContain('Add Account...');
    expect(labels).toContain('Manage Accounts...');
  });

  it('opens the native menu once from Enter and Space', () => {
    const { container } = renderSwitcher();
    const button = container.querySelector('.account-switcher');
    const enter = fireEvent.keyDown(button, { key: 'Enter' });
    expect(enter).toBe(false); // preventDefault, so the button doesn't also click
    fireEvent.keyDown(button, { key: ' ' });
    expect(popup.callCount).toBe(2);
  });
});

describe('Account sidebar slots', function () {
  afterEach(() => cleanup());

  // Plugins (activity) build sheets on the RootSidebar column and expect the account sidebar
  // and anything else registered there to render in it.
  it('renders components registered in the RootSidebar location', () => {
    const mode = WorkspaceStore.layoutMode();
    WorkspaceStore.defineSheet('AccountSidebarSlotSpec', {}, { [mode]: ['RootSidebar'] });
    const Fake = () => <div className="fake-root-sidebar-item">Fake</div>;
    Fake.displayName = 'FakeRootSidebarItem';
    ComponentRegistry.register(Fake, { location: WorkspaceStore.Location.RootSidebar });
    try {
      const { container } = render(
        <Sheet data={WorkspaceStore.Sheet.AccountSidebarSlotSpec} depth={0} />
      );
      const column = container.querySelector('.column-RootSidebar');
      expect(column).not.toBeNull();
      expect(column.querySelector('.fake-root-sidebar-item')).not.toBeNull();
    } finally {
      ComponentRegistry.unregister(Fake);
      WorkspaceStore.undefineSheet('AccountSidebarSlotSpec');
    }
  });

  // ExtensionRegistry.AccountSidebar: thread-snooze, send-reminders, activity and ai add
  // mailbox items through it.
  it('renders an item from a registered AccountSidebar extension', () => {
    const account: Account = AccountStore.accounts()[0];
    const inbox = new Folder({
      id: 'inbox-id',
      path: 'INBOX',
      role: 'inbox',
      accountId: account.id,
    });
    spyOn(CategoryStore, 'categories').andReturn([inbox]);
    spyOn(CategoryStore, 'standardCategories').andReturn([inbox]);
    spyOn(CategoryStore, 'userCategories').andReturn([]);
    spyOn(FocusedPerspectiveStore, 'sidebarAccountIds').andReturn([account.id]);

    const extension = {
      name: 'FakeSidebarExtension',
      sidebarItem: (accountIds: string[]) => ({
        id: 'FakeExtensionItem',
        name: 'Fake Extension Item',
        iconName: 'inbox.png',
        perspective: MailboxPerspective.forInbox(accountIds),
        insertAtTop: false,
      }),
    };
    ExtensionRegistry.AccountSidebar.register(extension);
    try {
      const { container } = render(<AccountSidebar />);
      act(() => {
        // The store listens to the registry; trigger once more in case it already ran.
        (SidebarStore as any)._updateSections();
      });
      expect(container.querySelector('.account-sidebar')).not.toBeNull();
      expect(container.textContent).toContain('Fake Extension Item');
    } finally {
      ExtensionRegistry.AccountSidebar.unregister(extension);
    }
  });
});
