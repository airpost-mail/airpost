import React from 'react';
import ReactDOM from 'react-dom';
import ReactTestUtils from 'react-dom/test-utils';
import { Thread } from '../../src/data/models/thread';
import * as Actions from '../../src/data/actions';
import CategoryStore from '../../src/data/stores/category';
import { TaskFactory } from '../../src/data/tasks/factory';
import { MailboxPerspective } from '../../src/data/mailbox-perspective';
import FocusedPerspectiveStore from '../../src/data/stores/focused-perspective';
import FocusedContentStore from '../../src/data/stores/focused-content';
import { Folder } from '../../src/data/models/folder';
import {
  ArchiveButton,
  ToggleStarredButton,
  ToggleUnreadButton,
  MarkAsSpamButton,
  DownButton,
} from '../../src/features/thread-list/toolbar-buttons';
import ThreadListStore from '../../src/features/thread-list/store';

const test_thread = new Thread({}).fromJSON({
  id: 'thread_12345',
  account_id: TEST_ACCOUNT_ID,
  subject: 'Subject 12345',
  starred: false,
});

const test_thread_starred = new Thread({}).fromJSON({
  id: 'thread_starred_12345',
  account_id: TEST_ACCOUNT_ID,
  subject: 'Subject 12345',
  starred: true,
});

describe('ThreadToolbarButtons', function () {
  beforeEach(function () {
    spyOn(Actions, 'queueTask');
    spyOn(Actions, 'queueTasks');
    spyOn(TaskFactory, 'taskForInvertingStarred').andCallThrough();
    spyOn(TaskFactory, 'taskForSettingUnread').andCallThrough();
  });

  describe('Markup', function () {
    it('draws the action buttons as kit toolbar icon buttons labelled by their action', function () {
      const button = ReactTestUtils.renderIntoDocument(
        <ToggleStarredButton items={[test_thread]} />
      ) as any;
      const node = ReactDOM.findDOMNode(button) as HTMLElement;
      expect(node.getAttribute('data-slot')).toBe('icon-button');
      expect(node.getAttribute('data-variant')).toBe('toolbar');
      expect(node.classList.contains('btn-toolbar')).toBe(true);
      expect(node.getAttribute('aria-label')).toBe('Star');
      expect(node.getAttribute('title')).toBeNull();
    });

    it('draws the thread arrows as kit icon buttons, disabled at the end of the list', function () {
      spyOn(FocusedContentStore, 'focusedId').andReturn('last');
      spyOn(ThreadListStore, 'dataSource').andReturn({
        count: () => 2,
        get: (idx) => ({ id: idx === 1 ? 'last' : 'first' }),
      } as any);
      spyOn(AppEnv.commands, 'dispatch');
      const container = document.createElement('div');
      ReactDOM.render(<DownButton />, container);
      const node = container.querySelector('button');
      expect(node.getAttribute('data-slot')).toBe('icon-button');
      expect(node.classList.contains('message-toolbar-arrow')).toBe(true);
      expect(node.classList.contains('down')).toBe(true);
      expect(node.classList.contains('disabled')).toBe(true);
      expect(node.disabled).toBe(true);
      expect(node.getAttribute('aria-label')).toBe('Next thread');
      ReactDOM.unmountComponentAtNode(container);
    });
  });

  describe('Starring', function () {
    it('stars a thread if the star button is clicked and thread is unstarred', function () {
      const starButton = ReactTestUtils.renderIntoDocument(
        <ToggleStarredButton items={[test_thread]} />
      ) as any;

      ReactTestUtils.Simulate.click(ReactDOM.findDOMNode(starButton) as HTMLElement);

      expect((TaskFactory.taskForInvertingStarred as any).mostRecentCall.args[0].threads).toEqual([
        test_thread,
      ]);
    });

    it('unstars a thread if the star button is clicked and thread is starred', function () {
      const starButton = ReactTestUtils.renderIntoDocument(
        <ToggleStarredButton items={[test_thread_starred]} />
      ) as any;

      ReactTestUtils.Simulate.click(ReactDOM.findDOMNode(starButton) as HTMLElement);

      expect((TaskFactory.taskForInvertingStarred as any).mostRecentCall.args[0].threads).toEqual([
        test_thread_starred,
      ]);
    });
  });

  describe('Archiving', function () {
    it('scopes the archive to the folder being viewed', function () {
      const sent = new Folder({
        id: 'sent-id',
        accountId: TEST_ACCOUNT_ID,
        role: 'sent',
        path: 'Sent',
      } as any);
      const perspective = MailboxPerspective.forCategories([sent]);
      spyOn(FocusedPerspectiveStore, 'current').andReturn(perspective);
      spyOn(perspective, 'canArchiveThreads').andReturn(true);
      spyOn(TaskFactory, 'tasksForArchiving').andReturn([]);
      spyOn(Actions, 'popSheet');

      const thread = new Thread({ id: 'thread-archive', accountId: TEST_ACCOUNT_ID });
      const button = ReactTestUtils.renderIntoDocument(<ArchiveButton items={[thread]} />) as any;
      ReactTestUtils.Simulate.click(ReactDOM.findDOMNode(button) as HTMLElement);

      const args = (TaskFactory.tasksForArchiving as any).mostRecentCall.args[0];
      expect(args.threads).toEqual([thread]);
      expect(args.perspective).toBe(perspective);
      expect(args.perspective.sourceFolderIdsForAccount(TEST_ACCOUNT_ID)).toEqual(['sent-id']);
    });
  });

  describe('Marking as unread', function () {
    let thread = null;
    let markUnreadBtn = null;

    beforeEach(function () {
      thread = new Thread({ id: 'thread-id-lol-123', accountId: TEST_ACCOUNT_ID, unread: false });
      markUnreadBtn = ReactTestUtils.renderIntoDocument(
        <ToggleUnreadButton items={[thread]} />
      ) as any;
    });

    it('queues a task to change unread status to true', function () {
      ReactTestUtils.Simulate.click(ReactDOM.findDOMNode(markUnreadBtn) as HTMLElement);
      expect((TaskFactory.taskForSettingUnread as any).mostRecentCall.args[0].threads).toEqual([
        thread,
      ]);
      expect(Actions.queueTask).toHaveBeenCalled();
    });

    it('returns to the thread list', function () {
      spyOn(Actions, 'popSheet');
      ReactTestUtils.Simulate.click(ReactDOM.findDOMNode(markUnreadBtn) as HTMLElement);
      expect(Actions.popSheet).toHaveBeenCalled();
    });
  });

  describe('Marking as spam', function () {
    let thread = null;
    let markSpamButton = null;

    describe('when the thread is already in spam', function () {
      beforeEach(function () {
        thread = new Thread({
          id: 'thread-id-lol-123',
          accountId: TEST_ACCOUNT_ID,
          folders: [{ role: 'spam' }],
        });
        markSpamButton = ReactTestUtils.renderIntoDocument(
          <MarkAsSpamButton items={[thread]} />
        ) as any;
      });

      it('queues a task to remove spam', function () {
        spyOn(TaskFactory, 'tasksForMarkingNotSpam');
        spyOn(CategoryStore, 'getSpamCategory').andReturn(thread.folders[0]);
        ReactTestUtils.Simulate.click(ReactDOM.findDOMNode(markSpamButton) as HTMLElement);
        expect((TaskFactory.tasksForMarkingNotSpam as any).mostRecentCall.args[0].threads).toEqual([
          thread,
        ]);
        expect(Actions.queueTasks).toHaveBeenCalled();
      });
    });

    describe('when the thread can be moved to spam', function () {
      beforeEach(function () {
        spyOn(MailboxPerspective.prototype, 'canMoveThreadsTo').andReturn(true);
        thread = new Thread({ id: 'thread-id-lol-123', accountId: TEST_ACCOUNT_ID, folders: [] });
        markSpamButton = ReactTestUtils.renderIntoDocument(
          <MarkAsSpamButton items={[thread]} />
        ) as any;
      });

      it('queues a task to mark as spam', function () {
        spyOn(TaskFactory, 'tasksForMarkingAsSpam');
        ReactTestUtils.Simulate.click(ReactDOM.findDOMNode(markSpamButton) as HTMLElement);
        expect(TaskFactory.tasksForMarkingAsSpam).toHaveBeenCalledWith({
          threads: [thread],
          source: 'Toolbar Button: Thread List',
        });
      });

      it('returns to the thread list', function () {
        spyOn(Actions, 'popSheet');
        ReactTestUtils.Simulate.click(ReactDOM.findDOMNode(markSpamButton) as HTMLElement);
        expect(Actions.popSheet).toHaveBeenCalled();
      });
    });
  });
});
