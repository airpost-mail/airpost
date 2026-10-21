// SPDX-License-Identifier: GPL-3.0-only
// The registry slots the message list keeps because plugins (or the documented plugin API) use
// them. Each spec registers a fake component and checks the slot renders it with its props.
import React from 'react';
import { render, cleanup } from '@testing-library/react';
import { Thread } from '../../src/data/models/thread';
import { Message } from '../../src/data/models/message';
import { Contact } from '../../src/data/models/contact';
import { File } from '../../src/data/models/file';
import ComponentRegistry from '../../src/packages/component-registry';
import * as ExtensionRegistry from '../../src/packages/extension-registry';
import FocusedContactsStore from '../../src/data/stores/focused-contacts';
import { MessageStore } from '../../src/data/stores/message';
import DraftStore from '../../src/data/drafts/store';
import WorkspaceStore from '../../src/data/stores/workspace';
import MessageItemBody from '../../src/features/message-list/message-item-body';
import MessageItemContainer from '../../src/features/message-list/message-item-container';
import MessageControls from '../../src/features/message-list/message-controls';
import MessageList from '../../src/features/message-list/message-list';
import { SidebarPluginContainer } from '../../src/features/message-list/sidebar-plugin-container';
import { SidebarParticipantPicker } from '../../src/features/message-list/sidebar-participant-picker';
import { MessageListHiddenMessagesToggle } from '../../src/features/message-list/hidden-messages-toggle';
import * as MessageListFeature from '../../src/features/message-list';
import { Menus } from '../../src/platform/app-api';

const proxyquire = require('proxyquire').noPreserveCache();

class StubBody extends React.Component {
  static displayName = 'StubBody';
  render() {
    return <div className="stub-body" />;
  }
}

// MessageItem with its body stubbed, so no message frame loads.
const MessageItem = proxyquire('../../src/features/message-list/message-item', {
  './message-item-body': { default: StubBody, __esModule: true },
}).default;

const thread = new Thread({ id: 'thread-1', subject: 'Hello', accountId: TEST_ACCOUNT_ID });
const sender = new Contact({ name: 'Ada', email: 'ada@example.com', accountId: TEST_ACCOUNT_ID });

const makeMessage = (props = {}) =>
  new Message({
    id: 'msg-1',
    headerMessageId: 'hmid-1',
    accountId: TEST_ACCOUNT_ID,
    threadId: thread.id,
    from: [sender],
    to: [sender],
    cc: [],
    bcc: [],
    replyTo: [],
    body: 'Hello there',
    date: new Date(1700000000000),
    draft: false,
    files: [],
    ...props,
  });

function fake(displayName: string) {
  const C = (props: any) => (
    <span className={`fake-${displayName}`} data-message={props.message && props.message.id}>
      {props.thread ? props.thread.id : ''}
    </span>
  );
  C.displayName = displayName;
  return C;
}

describe('Message list slots', function () {
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

  describe('in the message header', () => {
    it('renders MessageHeader, MessageHeaderStatus and MessageFooterStatus components', () => {
      register(fake('FakeMessageHeader'), { role: 'MessageHeader' });
      register(fake('FakeHeaderStatus'), { role: 'MessageHeaderStatus' });
      register(fake('FakeFooterStatus'), { role: 'MessageFooterStatus' });
      const message = makeMessage();
      const { container } = render(
        <MessageItem
          thread={thread}
          message={message}
          messages={[message]}
          collapsed={false}
          pending={false}
          isMostRecent
          className="message-item-wrap"
        />
      );
      const header = container.querySelector('.message-header');
      for (const name of ['FakeMessageHeader', 'FakeHeaderStatus']) {
        const el = header.querySelector(`.fake-${name}`);
        expect(el).not.toBeNull();
        expect(el.getAttribute('data-message')).toBe('msg-1');
        expect(el.textContent).toBe('thread-1');
      }
      expect(header.querySelector('.message-header-status .fake-FakeHeaderStatus')).not.toBeNull();
      const footer = container.querySelector('.message-footer-status .fake-FakeFooterStatus');
      expect(footer).not.toBeNull();
      expect(footer.getAttribute('data-message')).toBe('msg-1');
    });

    it('lists attachments directly, filtered by MessageView extensions', () => {
      const keep = new File({ id: 'f1', filename: 'keep.pdf', contentType: 'application/pdf' });
      const drop = new File({ id: 'f2', filename: 'drop.pdf', contentType: 'application/pdf' });
      const extension = {
        name: 'FakeFileFilter',
        filterMessageFiles: ({ files }) => files.filter((f) => f.id !== 'f2'),
      };
      ExtensionRegistry.MessageView.register(extension as any);
      try {
        const message = makeMessage({ files: [keep, drop] });
        const { container } = render(
          <MessageItem
            thread={thread}
            message={message}
            messages={[message]}
            collapsed={false}
            pending={false}
            isMostRecent
            className="message-item-wrap"
          />
        );
        const area = container.querySelector('.attachments-area');
        expect(area).not.toBeNull();
        expect(area.textContent).toContain('keep');
        expect(area.textContent).not.toContain('drop');
      } finally {
        ExtensionRegistry.MessageView.unregister(extension);
      }
    });
  });

  it('renders message:BodyHeader components above the body', () => {
    register(fake('FakeBodyHeader'), { role: 'message:BodyHeader' });
    const message = makeMessage();
    const { container } = render(<MessageItemBody message={message} downloads={{}} />);
    const el = container.querySelector('.fake-FakeBodyHeader');
    expect(el).not.toBeNull();
    expect(el.getAttribute('data-message')).toBe('msg-1');
  });

  it('renders MessageListHeaders components above the messages', () => {
    register(fake('FakeListHeader'), { role: 'MessageListHeaders' });
    spyOn(MessageStore, 'itemsLoading').andReturn(false);
    // No messages: the headers render above the list either way, and no message frame loads.
    spyOn(MessageStore, 'items').andReturn([]);
    spyOn(MessageStore, 'thread').andReturn(thread);
    spyOn(MessageStore, 'itemsExpandedState').andReturn({});
    const { container } = render(<MessageList />);
    const el = container.querySelector('.message-list-headers .fake-FakeListHeader');
    expect(el).not.toBeNull();
    expect(el.textContent).toBe('thread-1');
  });

  it('renders MessageListSidebar:ContactCard components for the focused contact', () => {
    const Card = (props: { contact: Contact }) => (
      <span className="fake-contact-card">{props.contact.email}</span>
    );
    Card.displayName = 'FakeContactCard';
    register(Card, { role: 'MessageListSidebar:ContactCard' });
    spyOn(FocusedContactsStore, 'focusedContact').andReturn(sender);
    spyOn(FocusedContactsStore, 'sortedContacts').andReturn([sender]);
    const { container } = render(<SidebarPluginContainer />);
    expect(container.querySelector('.sidebar-section.visible')).not.toBeNull();
    expect(container.querySelector('.sidebar-contact-card .fake-contact-card').textContent).toBe(
      'ada@example.com'
    );
  });

  it('renders the registered Composer for an expanded draft', () => {
    // A class, since MessageItemContainer gives the composer a ref.
    class Composer extends React.Component<{ headerMessageId: string; mode: string }> {
      static displayName = 'FakeComposer';
      render() {
        return (
          <span className="fake-composer">{`${this.props.headerMessageId} ${this.props.mode}`}</span>
        );
      }
    }
    spyOn(ComponentRegistry, 'findComponentsMatching').andCallFake(({ role }) =>
      role === 'Composer' ? [Composer] : []
    );
    spyOn(DraftStore, 'isSendingDraft').andReturn(false);
    const draft = makeMessage({ draft: true });
    const { container } = render(
      <MessageItemContainer
        thread={thread}
        message={draft}
        messages={[draft]}
        collapsed={false}
        isMostRecent
        isBeforeReplyArea={false}
        scrollTo={() => {}}
      />
    );
    expect(container.querySelector('.fake-composer').textContent).toBe('hmid-1 inline');
  });

  it('adds MessageActionMenuItem items to the actions menu', () => {
    const click = jasmine.createSpy('click');
    const ActionItem = {
      displayName: 'FakeActionItem',
      getMenuItem: (props: { message: Message }) => ({
        label: `Act on ${props.message.id}`,
        click,
      }),
    };
    register(ActionItem, { role: 'MessageActionMenuItem' });
    const popup = spyOn(Menus, 'popup').andReturn(Promise.resolve());
    const { container } = render(<MessageControls thread={thread} message={makeMessage()} />);
    (container.querySelector('.message-actions-ellipsis') as HTMLElement).click();
    expect(popup.callCount).toBe(1);
    const template = popup.calls[0].args[0];
    const item = template[template.length - 1];
    expect(template[template.length - 2].type).toBe('separator');
    expect(item.label).toBe('Act on msg-1');
    item.click();
    expect(click).toHaveBeenCalled();
  });

  it('registers the message list and its sidebar at their workspace locations', () => {
    spyOn(AppEnv, 'isMainWindow').andReturn(true);
    // Spec windows define no sheets, so the columns get stand-in locations.
    const store = WorkspaceStore as any;
    const savedLocation = store.Location;
    store.Location = {
      MessageList: { id: 'MessageList' },
      MessageListSidebar: { id: 'MessageListSidebar' },
    };
    const at = (location) => ComponentRegistry.findComponentsMatching({ location });
    const wasActive = at(WorkspaceStore.Location.MessageList).includes(MessageList);
    MessageListFeature.activate();
    try {
      expect(at(WorkspaceStore.Location.MessageList)).toContain(MessageList);
      expect(at(WorkspaceStore.Location.MessageListSidebar)).toContain(SidebarParticipantPicker);
      expect(at(WorkspaceStore.Location.MessageListSidebar)).toContain(SidebarPluginContainer);
      expect(ComponentRegistry.findComponentsMatching({ role: 'MessageListHeaders' })).toContain(
        MessageListHiddenMessagesToggle
      );
    } finally {
      MessageListFeature.deactivate();
    }
    expect(ComponentRegistry.findComponentsMatching({ role: 'MessageListHeaders' })).not.toContain(
      MessageListHiddenMessagesToggle
    );
    if (wasActive) MessageListFeature.activate();
    store.Location = savedLocation;
  });
});
