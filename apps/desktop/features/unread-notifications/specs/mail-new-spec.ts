// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import {
  Contact,
  Message,
  Thread,
  Folder,
  DatabaseStore,
  NativeNotifications,
} from 'mailspring-exports';

import { Notifier } from '../lib/main';
import { EngineEvents } from '../../../src/engine-events';
import { Feature } from '../../../src/shared/engine-protocol';

describe('UnreadNotifications on mail.new', function () {
  beforeEach(function () {
    const inbox = new Folder({ id: 'l1', role: 'inbox', path: 'Inbox' });
    this.thread = new Thread({ id: 'A', accountId: 'a', folders: [inbox] });
    const message = (id: string, extra = {}) =>
      new Message({
        id,
        unread: true,
        // After the notifier starts, which is the launch time it compares with.
        date: new Date(Date.now() + 60000),
        accountId: 'a',
        from: [new Contact({ name: 'Ben', email: 'ben@example.com' })],
        subject: `Hello ${id}`,
        threadId: 'A',
        version: 1,
        ...extra,
      });
    this.messages = {
      m1: message('m1'),
      m2: message('m2'),
      old: message('old', { date: new Date(2000, 1, 1) }),
    };
    spyOn(DatabaseStore, 'findAll').andCallFake((klass) => {
      if (klass === Message) {
        return Promise.resolve(Object.values(this.messages));
      }
      return Promise.resolve([this.thread]);
    });
    this.display = spyOn(NativeNotifications, 'displayNotification').andReturn(
      jasmine.createSpyObj('notification', ['close'])
    );
    spyOn(NativeNotifications, 'displaySummaryNotification').andReturn(Promise.resolve(null));

    this.engine = new EngineEvents();
    this.notifier = new Notifier(this.engine);
    this.mailNew = (messageId: string) =>
      this.engine.mailNew({ accountId: 'a', threadId: 'A', messageId });
    // The batch the timer starts, to await it.
    this.batch = null;
    const runBatch = this.notifier._onMailNewBatch.bind(this.notifier);
    this.notifier._onMailNewBatch = () => (this.batch = runBatch());
    this.fire = async () => {
      advanceClock(100);
      await this.batch;
    };
  });

  afterEach(function () {
    this.notifier.unlisten();
  });

  it('notifies once per message named by mail.new, after the batch delay', function () {
    waitsForPromise(async () => {
      this.engine.connected([Feature.MailNew], () => Promise.resolve(null));
      this.messages = { m1: this.messages.m1 };
      this.mailNew('m1');
      this.mailNew('m1');
      expect(DatabaseStore.findAll).not.toHaveBeenCalled();
      await this.fire();
      expect(this.display.callCount).toEqual(1);
      expect(this.display.mostRecentCall.args[0].messageId).toEqual('m1');

      // A repeat later is dropped too.
      this.mailNew('m1');
      await this.fire();
      expect(this.display.callCount).toEqual(1);
    });
  });

  it('keeps the launch-time rule', function () {
    waitsForPromise(async () => {
      this.engine.connected([Feature.MailNew], () => Promise.resolve(null));
      this.messages = { old: this.messages.old };
      this.mailNew('old');
      await this.fire();
      expect(NativeNotifications.displayNotification).not.toHaveBeenCalled();
    });
  });

  it('ignores message deltas when the engine sends mail.new, and uses them otherwise', function () {
    waitsForPromise(async () => {
      const delta = {
        objectClass: Message.name,
        objects: [this.messages.m2],
        objectsRawJSON: [{ id: 'm2', headersSyncComplete: true }],
      };
      this.engine.connected([Feature.MailNew], () => Promise.resolve(null));
      await this.notifier._onDatabaseChanged(delta as any);
      expect(NativeNotifications.displayNotification).not.toHaveBeenCalled();

      this.engine.gone();
      await this.notifier._onDatabaseChanged(delta as any);
      expect(this.display.callCount).toEqual(1);
    });
  });

  it('does nothing when notifications are off', function () {
    waitsForPromise(async () => {
      spyOn(AppEnv.config, 'get').andCallFake((key) =>
        key === 'core.notifications.enabled' ? false : undefined
      );
      this.engine.connected([Feature.MailNew], () => Promise.resolve(null));
      this.mailNew('m1');
      await this.fire();
      expect(DatabaseStore.findAll).not.toHaveBeenCalled();
      expect(NativeNotifications.displayNotification).not.toHaveBeenCalled();
    });
  });
});
