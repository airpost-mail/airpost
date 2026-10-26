import React from 'react';
import ReactDOM from 'react-dom';
import ReactTestUtils from 'react-dom/test-utils';
import MovePickerPopover from '../../src/features/category-picker/move-picker-popover';

import { Folder } from '../../src/data/models/folder';
import { Label } from '../../src/data/models/label';
import { Thread } from '../../src/data/models/thread';
import * as Actions from '../../src/data/actions';
import { AccountStore } from '../../src/data/stores/account';
import CategoryStore from '../../src/data/stores/category';
import FocusedPerspectiveStore from '../../src/data/stores/focused-perspective';
import { MailboxPerspective } from '../../src/data/mailbox-perspective';
import TestUtils from '../support/test-utils';
import TaskQueue from '../../src/data/stores/task-queue';
import EngineEvents from '../../src/engine/client/events';
import { IntentTask } from '../../src/data/tasks/intent';

import { Categories } from '../../src/data/query/observables';

describe('MovePickerPopover', function () {
  beforeEach(() => (CategoryStore._categoryCache = {}));

  const setupFor = function () {
    this.account = {
      id: TEST_ACCOUNT_ID,
    };

    this.inboxCategory = new Folder({
      id: 'id-123',
      role: 'inbox',
      path: 'INBOX',
      accountId: TEST_ACCOUNT_ID,
    });
    this.archiveCategory = new Folder({
      id: 'id-456',
      role: 'archive',
      path: 'ArCHIVe',
      accountId: TEST_ACCOUNT_ID,
    });
    this.userCategory = new Folder({
      id: 'id-789',
      role: null,
      path: 'MyCategory',
      accountId: TEST_ACCOUNT_ID,
    });

    const observable = TestUtils.mockObservable(
      [this.inboxCategory, this.archiveCategory, this.userCategory],
      {}
    );
    (observable as any).sort = () => observable;

    spyOn(Categories, 'forAccount').andReturn(observable);
    spyOn(CategoryStore, 'getCategoryByRole').andReturn(this.inboxCategory);
    spyOn(AccountStore, 'accountForItems').andReturn(this.account);
    spyOn(Actions, 'closePopover');

    // By default we're going to set to "inbox". This has implications for
    // what categories get filtered out of the list.
    spyOn(FocusedPerspectiveStore, 'current').andCallFake(() => {
      return MailboxPerspective.forCategory(this.inboxCategory);
    });
  };

  const setupForCreateNew = function () {
    setupFor.call(this);

    this.testThread = new Thread({
      id: 't1',
      subject: 'fake',
      accountId: TEST_ACCOUNT_ID,
      categories: [],
      folders: [this.inboxCategory],
      labels: [],
    });
    this.picker = ReactTestUtils.renderIntoDocument(
      <MovePickerPopover threads={[this.testThread]} account={this.account} />
    );
  };

  describe('when using folders', function () {
    beforeEach(function () {
      setupFor.call(this);

      this.testThread = new Thread({
        id: 't1',
        subject: 'fake',
        accountId: TEST_ACCOUNT_ID,
        categories: [],
        folders: [],
        labels: [],
      });
      this.picker = ReactTestUtils.renderIntoDocument(
        <MovePickerPopover threads={[this.testThread]} account={this.account} />
      );
    });

    it('lists the desired categories', function () {
      const data = this.picker.state.categoryData;
      // NOTE: The inbox category is not included here because it's the
      // currently focused category, which gets filtered out of the list.
      expect(data.length).toBe(3);

      expect(data[0].id).toBe('id-456');
      expect(data[0].category.role).toBe('archive');
      expect(data[0].category).toBe(this.archiveCategory);

      expect(data[1].divider).toBe(true);
      expect(data[1].id).toBe('category-divider');

      expect(data[2].id).toBe('id-789');
      expect(data[2].category.role).toBeNull();
      expect(data[2].category).toBe(this.userCategory);
    });
  });

  describe("'create new' item", function () {
    beforeEach(function () {
      setupForCreateNew.call(this);
    });

    it('is not visible when the search box is empty', function () {
      const count = ReactTestUtils.scryRenderedDOMComponentsWithClass(
        this.picker,
        'category-create-new'
      ).length;
      expect(count).toBe(0);
    });

    it('is visible when the search box has text', function () {
      const inputNode = ReactDOM.findDOMNode(
        ReactTestUtils.scryRenderedDOMComponentsWithTag(this.picker, 'input')[0]
      );
      ReactTestUtils.Simulate.change(
        inputNode as Element,
        { target: { value: 'calendar' } } as any
      );
      const count = ReactTestUtils.scryRenderedDOMComponentsWithClass(
        this.picker,
        'category-create-new'
      ).length;
      expect(count).toBe(1);
    });

    it("shows folder icon if we're using exchange", function () {
      const inputNode = ReactDOM.findDOMNode(
        ReactTestUtils.scryRenderedDOMComponentsWithTag(this.picker, 'input')[0]
      );
      ReactTestUtils.Simulate.change(
        inputNode as Element,
        { target: { value: 'calendar' } } as any
      );
      const count = ReactTestUtils.scryRenderedDOMComponentsWithClass(
        this.picker,
        'category-create-new-folder'
      ).length;
      expect(count).toBe(1);
    });
  });

  describe('_onSelectCategory', function () {
    beforeEach(function () {
      setupForCreateNew.call(this);
      spyOn(Actions, 'queueTask');
    });

    it('closes the popover', function () {
      this.picker._onSelectCategory({ category: this.archiveCategory });
      expect(Actions.closePopover).toHaveBeenCalled();
    });

    // A move is the engine's threads.move: to a folder it takes the copies the view shows, to
    // a Gmail label it replaces the threads' labels (tern-tasks intents.rs).
    describe('when selecting a folder', () =>
      it('queues the move intent', function () {
        this.picker._onSelectCategory({ category: this.archiveCategory });
        expect(Actions.queueTask).toHaveBeenCalled();
        const task = (Actions.queueTask as any).calls[0].args[0];
        expect(task instanceof IntentTask).toBe(true);
        expect(task.intent).toBe('threads.move');
        expect(task.params.to).toBe(this.archiveCategory.id);
        expect(task.threadIds).toEqual([this.testThread.id]);
      }));

    describe('when selecting a label', () =>
      it('queues the move intent with the label', function () {
        const labelCategory = new Label({
          id: 'lbl-1',
          path: 'MyLabel',
          accountId: TEST_ACCOUNT_ID,
        });
        this.picker._onSelectCategory({ category: labelCategory });
        const task = (Actions.queueTask as any).calls[0].args[0];
        expect(task.intent).toBe('threads.move');
        expect(task.params.to).toBe('lbl-1');
        expect(task.description()).toBe('Moved to MyLabel');
      }));

    describe('when selecting a new category', function () {
      beforeEach(function () {
        this.input = { newCategoryItem: true };
        this.picker.setState({ searchValue: 'teSTing!' });
      });

      it('creates it with categories.create, then moves the threads there', function () {
        const requests: any[] = [];
        EngineEvents.features = new Set(['intents']);
        EngineEvents._request = (method, params) => {
          requests.push([method, params]);
          return Promise.resolve({ taskIds: ['task-1'] });
        };
        spyOn(TaskQueue, 'waitForPerformRemote').andCallFake((task) => {
          expect(task.id).toBe('task-1');
          return Promise.resolve({ id: 'task-1', created: { id: 'created-1' } });
        });

        this.picker._onSelectCategory(this.input);

        waitsFor(() => (Actions.queueTask as any).callCount > 0);

        runs(function () {
          expect(requests).toEqual([
            ['categories.create', { accountId: TEST_ACCOUNT_ID, name: 'teSTing!' }],
          ]);
          const moveTask = (Actions.queueTask as any).calls[0].args[0];
          expect(moveTask.intent).toBe('threads.move');
          expect(moveTask.params.to).toBe('created-1');
          expect(moveTask.threadIds).toEqual([this.testThread.id]);
          EngineEvents.features = null;
          EngineEvents._request = null;
        });
      });
    });
  });
});
