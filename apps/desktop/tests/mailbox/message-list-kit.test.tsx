// SPDX-License-Identifier: GPL-3.0-only
// The message list's controls on the UI kit: the subject line icons, find in thread, the
// message header controls and the sidebar's contact picker.
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import * as Actions from '../../src/data/actions';
import { Contact } from '../../src/data/models/contact';
import { File } from '../../src/data/models/file';
import FocusedContactsStore from '../../src/data/stores/focused-contacts';
import { Message } from '../../src/data/models/message';
import SearchableComponentStore from '../../src/data/stores/searchable-component';
import { Thread } from '../../src/data/models/thread';
import { SubjectLineIcons } from '../../src/features/message-list/subject-line-icons';
import FindInThread from '../../src/features/message-list/find-in-thread';
import MessageControls from '../../src/features/message-list/message-controls';
import { SidebarParticipantPicker } from '../../src/features/message-list/sidebar-participant-picker';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait } from '../support/kit-helpers';
import { Menus } from '../../src/platform/app-api';

const proxyquire = require('proxyquire').noPreserveCache();

class StubBody extends React.Component {
  static displayName = 'StubBody';
  render() {
    return <div />;
  }
}
const MessageItem = proxyquire('../../src/features/message-list/message-item', {
  './message-item-body': { default: StubBody, __esModule: true },
}).default;

const thread = new Thread({ id: 'thread-1', subject: 'Hello', accountId: TEST_ACCOUNT_ID });
const ada = new Contact({ name: 'Ada', email: 'ada@example.com', accountId: TEST_ACCOUNT_ID });
const bob = new Contact({ name: 'Bob', email: 'bob@example.com', accountId: TEST_ACCOUNT_ID });

const makeMessage = (props = {}) =>
  new Message({
    id: 'msg-1',
    headerMessageId: 'hmid-1',
    accountId: TEST_ACCOUNT_ID,
    threadId: thread.id,
    from: [ada],
    to: [bob],
    cc: [],
    bcc: [],
    replyTo: [],
    body: 'Hello there',
    date: new Date(1700000000000),
    draft: false,
    files: [],
    ...props,
  });

const iconButtons = (root: Element) =>
  Array.from(root.querySelectorAll('[data-slot="icon-button"]')) as HTMLButtonElement[];

describe('Message list on the UI kit', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  describe('SubjectLineIcons', () => {
    const renderIcons = (props = {}) => {
      const handlers = {
        onPrint: jasmine.createSpy('onPrint'),
        onPopIn: jasmine.createSpy('onPopIn'),
        onPopOut: jasmine.createSpy('onPopOut'),
        onToggleAllExpanded: jasmine.createSpy('onToggleAllExpanded'),
      };
      const result = render(
        <SubjectLineIcons
          canCollapse
          hasCollapsedItems
          messageIds={['msg-1']}
          {...handlers}
          {...props}
        />
      );
      return { ...result, handlers };
    };

    it('renders icon buttons named by their labels', () => {
      const { container } = renderIcons();
      const wrap = container.querySelector('.message-icons-wrap');
      expect(wrap.querySelector('[role="button"]')).toBeNull();
      const labels = iconButtons(wrap).map((b) => b.getAttribute('aria-label'));
      expect(labels).toEqual([
        'Expand All',
        'Print Thread',
        'Show Original Colors',
        'Popout thread',
      ]);
      for (const button of iconButtons(wrap)) {
        expect(button.tagName).toBe('BUTTON');
        expect(button.hasAttribute('title')).toBe(false);
        expect(button.querySelector('img.content-mask')).not.toBeNull();
      }
    });

    it('hides the original colours toggle while emails are not darkened', () => {
      const { container } = renderIcons();
      const toggle = container.querySelector('.original-colors-toggle') as HTMLElement;
      expect(toggle.style.display).toBe('none');
    });

    it('calls its handlers on click', () => {
      const { container, handlers } = renderIcons({ hasCollapsedItems: false });
      const byLabel = (label: string) =>
        container.querySelector(`[aria-label="${label}"]`) as HTMLButtonElement;
      fireEvent.click(byLabel('Collapse All'));
      fireEvent.click(byLabel('Print Thread'));
      fireEvent.click(byLabel('Popout thread'));
      expect(handlers.onToggleAllExpanded).toHaveBeenCalled();
      expect(handlers.onPrint).toHaveBeenCalled();
      expect(handlers.onPopOut).toHaveBeenCalled();
    });

    it('shows the label as a tooltip on focus', async () => {
      const { container } = renderIcons();
      (container.querySelector('[aria-label="Print Thread"]') as HTMLElement).focus();
      await wait(20);
      const tooltip = getPortalContainer().querySelector('[data-slot="tooltip-content"]');
      expect(tooltip).not.toBeNull();
      expect(tooltip.textContent).toContain('Print Thread');
    });
  });

  describe('FindInThread', () => {
    let searchData: { searchTerm: string | null; globalIndex: number; resultsLength: number };

    beforeEach(() => {
      searchData = { searchTerm: 'hello', globalIndex: null, resultsLength: 0 };
      spyOn(SearchableComponentStore, 'getCurrentSearchData').andCallFake(() => searchData);
      spyOn(Actions, 'findInThread');
      spyOn(Actions, 'nextSearchResult');
      spyOn(Actions, 'previousSearchResult');
    });

    it('uses the kit text field and icon buttons', () => {
      const { container } = render(<FindInThread />);
      const input = container.querySelector('input') as HTMLInputElement;
      expect(input.getAttribute('data-slot')).toBe('input');
      expect(input.value).toBe('hello');
      const buttons = iconButtons(container);
      expect(buttons.map((b) => b.getAttribute('aria-label'))).toEqual([
        'Previous result',
        'Next result',
        'Close',
      ]);
      expect(buttons.every((b) => b.classList.contains('btn-find-in-thread'))).toBe(true);
      expect(container.querySelector('.btn:not([data-slot])')).toBeNull();
    });

    it('disables result navigation without results', () => {
      const { container } = render(<FindInThread />);
      const [prev, next, close] = iconButtons(container);
      expect(prev.disabled).toBe(true);
      expect(next.disabled).toBe(true);
      expect(close.disabled).toBe(false);
      fireEvent.click(close);
      expect(Actions.findInThread).toHaveBeenCalledWith(null);
    });

    it('moves between results and shows the position', () => {
      searchData = { searchTerm: 'hello', globalIndex: 1, resultsLength: 4 };
      const { container } = render(<FindInThread />);
      const [prev, next] = iconButtons(container);
      expect(prev.disabled).toBe(false);
      fireEvent.click(next);
      fireEvent.click(prev);
      expect(Actions.nextSearchResult).toHaveBeenCalled();
      expect(Actions.previousSearchResult).toHaveBeenCalled();
      expect(container.querySelector('.selection-progress').textContent).toBe('2 of 4');
    });
  });

  describe('MessageControls', () => {
    it('opens the native actions menu from an icon button', () => {
      const popup = spyOn(Menus, 'popup').andReturn(Promise.resolve());
      const { container } = render(<MessageControls thread={thread} message={makeMessage()} />);
      const ellipsis = container.querySelector('.message-actions-ellipsis') as HTMLButtonElement;
      expect(ellipsis.getAttribute('data-slot')).toBe('icon-button');
      expect(ellipsis.getAttribute('aria-label')).toBe('More Actions');
      expect(ellipsis.tabIndex).toBe(-1);
      fireEvent.click(ellipsis);
      expect(popup.callCount).toBe(1);
      const labels = popup.calls[0].args[0].map((i) => i.label);
      expect(labels).toContain('Show Original');
      expect(labels).toContain('Download as .eml');
    });
  });

  describe('MessageItem header', () => {
    const renderItem = (message: Message) =>
      render(
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

    it('toggles detailed headers from an icon button', () => {
      spyOn(AppEnv.config, 'get').andCallFake((key) =>
        key === 'core.reading.detailedHeaders' ? false : undefined
      );
      const { container } = renderItem(makeMessage());
      const toggle = container.querySelector('.header-toggle-control') as HTMLButtonElement;
      expect(toggle.getAttribute('data-slot')).toBe('icon-button');
      expect(toggle.classList.contains('inactive')).toBe(true);
      expect(toggle.getAttribute('aria-label')).toBe('Show Details');
      fireEvent.click(toggle);
      expect(toggle.classList.contains('inactive')).toBe(false);
      expect(toggle.getAttribute('aria-label')).toBe('Hide Details');
      expect(container.querySelector('.message-participants.collapsed')).toBeNull();
      fireEvent.click(toggle);
      expect(toggle.classList.contains('inactive')).toBe(true);
    });

    it('downloads all attachments from a kit button', () => {
      spyOn(Actions, 'fetchAndSaveAllFiles');
      const files = [
        new File({ id: 'f1', filename: 'a.pdf', contentType: 'application/pdf' }),
        new File({ id: 'f2', filename: 'b.pdf', contentType: 'application/pdf' }),
      ];
      const { container } = renderItem(makeMessage({ files }));
      const button = container.querySelector('.download-all-action') as HTMLButtonElement;
      expect(button.tagName).toBe('BUTTON');
      expect(button.getAttribute('data-slot')).toBe('button');
      expect(button.textContent).toBe('Download All');
      fireEvent.click(button);
      expect(Actions.fetchAndSaveAllFiles).toHaveBeenCalledWith(files);
    });
  });

  describe('SidebarParticipantPicker', () => {
    beforeEach(() => {
      spyOn(FocusedContactsStore, 'sortedContacts').andReturn([ada, bob]);
      spyOn(FocusedContactsStore, 'focusedContact').andReturn(ada);
      spyOn(Actions, 'focusContact');
    });

    it('picks the focused contact from a kit select', () => {
      const { container } = render(<SidebarParticipantPicker />);
      expect(container.querySelector('select')).toBeNull();
      const trigger = container.querySelector('[data-slot="select-trigger"]') as HTMLElement;
      expect(trigger.tabIndex).toBe(-1);
      expect(trigger.getAttribute('aria-label')).toBe('Contact');
      expect(trigger.textContent).toContain('Ada');
      fireEvent.keyDown(trigger, { key: 'Enter' });
      const options = Array.from(
        getPortalContainer().querySelectorAll('[data-slot="select-item"]')
      ) as HTMLElement[];
      expect(options.length).toBe(2);
      const bobOption = options.find((o) => o.textContent.includes('Bob'));
      fireEvent.keyDown(bobOption, { key: 'Enter' });
      expect(Actions.focusContact).toHaveBeenCalledWith(bob);
    });
  });
});
