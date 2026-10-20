// SPDX-License-Identifier: GPL-3.0-only
// The registry slots the thread list, draft list and sheet toolbar keep because plugins (or the
// documented plugin API) use them. Each spec registers a fake component and checks the slot
// renders it with its props.
import React from 'react';
import { render, cleanup } from '@testing-library/react';
import {
  Rx,
  Thread,
  Message,
  ComponentRegistry,
  ExtensionRegistry,
  FocusedContentStore,
  FocusedPerspectiveStore,
  WorkspaceStore,
} from 'mailspring-exports';
import ThreadListStore from '../../src/features/thread-list/store';
import ThreadListToolbar from '../../src/features/thread-list/toolbar';
import MessageListToolbar from '../../src/features/thread-list/message-list-toolbar';
import ThreadListIcon from '../../src/features/thread-list/icon';
import * as ThreadColumns from '../../src/features/thread-list/columns';
import DraftListStore from '../../src/features/draft-list/store';
import DraftListToolbar from '../../src/features/draft-list/toolbar';
import * as DraftColumns from '../../src/features/draft-list/columns';
import Toolbar from '../../src/components/sheets/toolbar';

const proxyquire = require('proxyquire').noPreserveCache();

class StubThreadList extends React.Component {
  static displayName = 'ThreadList';
  render() {
    return <div className="stub-thread-list" />;
  }
}

// The vertical layout with the thread list stubbed, so no list data source loads.
const ThreadListVertical = proxyquire('../../src/features/thread-list/vertical', {
  './thread-list': { default: StubThreadList, __esModule: true },
}).default;

const thread = new Thread({
  id: 'thread-1',
  subject: 'Hello',
  accountId: TEST_ACCOUNT_ID,
  starred: false,
  unread: false,
  labels: [],
  folders: [],
});

// The thread list's data source attaches the messages; the row cells read them.
(thread as any).__messages = [];

const draft = new Message({
  id: 'draft-1',
  headerMessageId: 'hmid-draft-1',
  accountId: TEST_ACCOUNT_ID,
  subject: 'Draft',
  draft: true,
  from: [],
  to: [],
  cc: [],
  bcc: [],
  files: [],
  body: '',
});

function fake(displayName: string) {
  const C = (props: any) => (
    <span className={`fake-${displayName}`}>
      {(props.thread && props.thread.id) ||
        (props.draft && props.draft.id) ||
        (props.items || []).map((i) => i.id).join(',')}
    </span>
  );
  C.displayName = displayName;
  C.containerRequired = false;
  return C;
}

const renderCells = (columns: any[], item: any) =>
  render(
    <div>
      {columns.map((column, idx) => (
        <div key={idx} className={`cell-${column.name}`}>
          {column.resolver(item)}
        </div>
      ))}
    </div>
  );

describe('Thread list slots', function () {
  const registered: any[] = [];
  const register = (component: any, options: any) => {
    ComponentRegistry.register(component, options);
    registered.push(component);
  };

  beforeEach(() => {
    spyOn(FocusedPerspectiveStore, 'current').andReturn({
      isSent: () => false,
      isInbox: () => true,
      canArchiveThreads: () => true,
      canMoveThreadsTo: () => true,
      categories: () => [],
      accountIds: [TEST_ACCOUNT_ID],
    } as any);
  });

  afterEach(() => {
    cleanup();
    registered.splice(0).forEach((c) => ComponentRegistry.unregister(c));
  });

  describe('in the thread toolbars', () => {
    beforeEach(() => {
      spyOn(ThreadListStore, 'selectionObservable').andReturn(Rx.Observable.just([thread]));
      spyOn(ThreadListStore, 'dataSource').andReturn({ selection: { clear() {} } } as any);
    });

    it('renders ThreadActionsToolbarButton and ThreadList:ThreadActionsToolbarButton above the list', () => {
      register(fake('FakeThreadAction'), { role: 'ThreadActionsToolbarButton' });
      register(fake('FakeListAction'), { role: 'ThreadList:ThreadActionsToolbarButton' });
      register(fake('FakeMessageAction'), { role: 'MessageList:ThreadActionsToolbarButton' });
      const { container } = render(<ThreadListToolbar />);
      expect(container.querySelector('.fake-FakeThreadAction').textContent).toBe('thread-1');
      expect(container.querySelector('.fake-FakeListAction').textContent).toBe('thread-1');
      expect(container.querySelector('.fake-FakeMessageAction')).toBeNull();
    });

    it('renders ThreadActionsToolbarButton and MessageList:ThreadActionsToolbarButton above the message', () => {
      spyOn(FocusedContentStore, 'focused').andReturn(thread);
      register(fake('FakeThreadAction'), { role: 'ThreadActionsToolbarButton' });
      register(fake('FakeMessageAction'), { role: 'MessageList:ThreadActionsToolbarButton' });
      register(fake('FakeListAction'), { role: 'ThreadList:ThreadActionsToolbarButton' });
      const { container } = render(<MessageListToolbar />);
      expect(container.querySelector('.fake-FakeThreadAction').textContent).toBe('thread-1');
      expect(container.querySelector('.fake-FakeMessageAction').textContent).toBe('thread-1');
      expect(container.querySelector('.fake-FakeListAction')).toBeNull();
    });
  });

  describe('in the thread rows', () => {
    it('renders ThreadListIcon, ThreadListTimestamp and ThreadListQuickAction components', () => {
      register(fake('FakeIcon'), { role: 'ThreadListIcon' });
      register(fake('FakeTimestamp'), { role: 'ThreadListTimestamp' });
      register(fake('FakeQuickAction'), { role: 'ThreadListQuickAction' });
      const { container } = renderCells(ThreadColumns.Wide, thread);
      expect(container.querySelector('.cell-★ .fake-FakeIcon').textContent).toBe('thread-1');
      expect(container.querySelector('.cell-Date .fake-FakeTimestamp').textContent).toBe(
        'thread-1'
      );
      const actions = container.querySelector('.cell-HoverActions');
      expect(actions.querySelector('.fake-FakeQuickAction').textContent).toBe('thread-1');
      // The built-in quick actions render beside the injected ones.
      expect(actions.querySelector('.action-archive[data-slot="button"]')).not.toBeNull();
      expect(actions.querySelector('.action-trash[data-slot="button"]')).not.toBeNull();
    });

    it('renders ThreadListIcon and ThreadListTimestamp components in the narrow layout', () => {
      register(fake('FakeIcon'), { role: 'ThreadListIcon' });
      register(fake('FakeTimestamp'), { role: 'ThreadListTimestamp' });
      const { container } = renderCells(ThreadColumns.Narrow, thread);
      expect(container.querySelector('.icons-column .fake-FakeIcon').textContent).toBe('thread-1');
      expect(container.querySelector('.fake-FakeTimestamp').textContent).toBe('thread-1');
    });

    it('falls back to the built-in timestamp when nothing is registered', () => {
      const { container } = renderCells(ThreadColumns.Wide, thread);
      expect(container.querySelector('.cell-Date .timestamp')).not.toBeNull();
    });

    it('draws the icon with the classes ExtensionRegistry.ThreadList extensions give', () => {
      const extension = {
        name: 'FakeIconClasses',
        cssClassNamesForThreadListIcon: () => 'fake-icon-class',
      };
      ExtensionRegistry.ThreadList.register(extension as any);
      try {
        const { container } = render(<ThreadListIcon thread={thread as any} />);
        const icon = container.querySelector('.thread-icon');
        expect(icon.getAttribute('data-slot')).toBe('button');
        expect(icon.classList.contains('fake-icon-class')).toBe(true);
        expect(icon.getAttribute('aria-label')).toBe('Star');
      } finally {
        ExtensionRegistry.ThreadList.unregister(extension as any);
      }
    });
  });

  describe('in the vertical layout', () => {
    // Spec windows define no sheets, so the message list gets a stand-in location.
    const store = WorkspaceStore as any;
    let savedLocation;

    beforeEach(() => {
      spyOn(AppEnv, 'getThreadListVerticalHeight').andReturn(300);
      savedLocation = store.Location;
      store.Location = {
        MessageList: { id: 'MessageList', Toolbar: { id: 'MessageList:Toolbar' } },
      };
    });

    afterEach(() => {
      store.Location = savedLocation;
    });

    it('renders the thread list directly and the MessageList location and its toolbar', () => {
      register(fake('FakeMessagePane'), {
        location: WorkspaceStore.Location.MessageList,
        modes: ['split'],
      });
      register(fake('FakeMessageToolbar'), {
        location: WorkspaceStore.Location.MessageList.Toolbar,
        modes: ['split'],
      });
      const { container } = render(<ThreadListVertical />);
      expect(container.querySelector('.stub-thread-list')).not.toBeNull();
      const pane = container.querySelector('.thread-list-vertical');
      expect(pane.querySelector('.sheet-toolbar .fake-FakeMessageToolbar')).not.toBeNull();
      expect(pane.querySelector('.fake-FakeMessagePane')).not.toBeNull();
    });
  });
});

describe('Draft list slots', function () {
  const registered: any[] = [];
  const register = (component: any, options: any) => {
    ComponentRegistry.register(component, options);
    registered.push(component);
  };

  afterEach(() => {
    cleanup();
    registered.splice(0).forEach((c) => ComponentRegistry.unregister(c));
  });

  it('renders the built-in delete button and DraftActionsToolbarButton components', () => {
    const selection = { items: () => [draft], clear: jasmine.createSpy('clear') };
    spyOn(DraftListStore, 'selectionObservable').andReturn(Rx.Observable.just([draft]));
    spyOn(DraftListStore, 'dataSource').andReturn({ selection } as any);
    register(fake('FakeDraftAction'), { role: 'DraftActionsToolbarButton' });
    const { container } = render(<DraftListToolbar />);
    expect(container.querySelector('.fake-FakeDraftAction').textContent).toBe('draft-1');
    const remove = container.querySelector('[data-slot="icon-button"][aria-label="Delete"]');
    expect(remove).not.toBeNull();
    expect(remove.parentElement).toBe(
      container.querySelector('.fake-FakeDraftAction').parentElement
    );
  });

  it('renders DraftList:DraftStatus components in the status column', () => {
    register(fake('FakeDraftStatus'), { role: 'DraftList:DraftStatus' });
    const { container } = renderCells(DraftColumns.Wide, draft);
    expect(container.querySelector('.cell-State .fake-FakeDraftStatus').textContent).toBe(
      'draft-1'
    );
  });
});

describe('Sheet toolbar slots', function () {
  const registered: any[] = [];
  const register = (component: any, options: any) => {
    ComponentRegistry.register(component, options);
    registered.push(component);
  };
  const columnA = { id: 'FakeColumnA', Toolbar: { id: 'FakeColumnA:Toolbar' } };
  const columnB = { id: 'FakeColumnB', Toolbar: { id: 'FakeColumnB:Toolbar' } };
  const sheet = {
    id: 'FakeSheet',
    columns: { list: [columnA, columnB] },
    Toolbar: { Left: { id: 'FakeSheet:Toolbar:Left' }, Right: { id: 'FakeSheet:Toolbar:Right' } },
    Header: { id: 'FakeSheet:Header' },
    Footer: { id: 'FakeSheet:Footer' },
  } as any;

  beforeEach(() => {
    spyOn(WorkspaceStore, 'layoutMode').andReturn('list');
    spyOn(WorkspaceStore, 'isLocationHidden').andReturn(false);
  });

  afterEach(() => {
    cleanup();
    registered.splice(0).forEach((c) => ComponentRegistry.unregister(c));
  });

  it('renders column toolbar, sheet and global toolbar components in their columns', () => {
    register(fake('FakeColumnA'), { location: columnA.Toolbar });
    register(fake('FakeColumnB'), { location: columnB.Toolbar });
    register(fake('FakeSheetLeft'), { location: sheet.Toolbar.Left });
    register(fake('FakeSheetRight'), { location: sheet.Toolbar.Right });
    register(fake('FakeGlobalLeft'), { location: WorkspaceStore.Sheet.Global.Toolbar.Left });
    register(fake('FakeGlobalRight'), { location: WorkspaceStore.Sheet.Global.Toolbar.Right });
    const { container } = render(<Toolbar data={sheet} depth={0} />);
    const first = container.querySelector('[data-column="0"]');
    const last = container.querySelector('[data-column="1"]');
    for (const name of ['FakeColumnA', 'FakeSheetLeft', 'FakeGlobalLeft']) {
      expect(first.querySelector(`.fake-${name}`)).not.toBeNull();
    }
    for (const name of ['FakeColumnB', 'FakeSheetRight', 'FakeGlobalRight']) {
      expect(last.querySelector(`.fake-${name}`)).not.toBeNull();
    }
    expect(first.className).toBe('toolbar-FakeColumnA');
  });

  it('shows the kit back button on a pushed sheet', () => {
    spyOn(FocusedPerspectiveStore, 'current').andReturn({ name: 'INBOX' } as any);
    const { container } = render(<Toolbar data={sheet} depth={1} />);
    const back = container.querySelector('[data-column="0"] .item-back');
    expect(back.getAttribute('data-slot')).toBe('button');
    expect(back.getAttribute('aria-label')).toBe('Return to Inbox');
    expect(back.querySelector('.item-back-title').textContent).toBe('Inbox');
  });
});
