import * as Actions from '../../src/data/actions';
import UnreadQuerySubscription from '../../src/data/query/unread-subscription';
import { QuerySubscription } from '../../src/data/query/subscription';
import { Thread } from '../../src/data/models/thread';
import { Folder } from '../../src/data/models/folder';
import { ChangeUnreadTask } from '../../src/data/tasks/change-unread';
import { ChangeFolderTask } from '../../src/data/tasks/change-folder';
import { DatabaseChangeRecord } from '../../src/data/database/change-record';
import { forEachTransport, settle } from '../support/mock-engine';

const thread = new Thread({ id: 't1', accountId: 'a1' });
const markRead = () =>
  Actions.queueTask(new ChangeUnreadTask({ threads: [thread], unread: false, source: 'spec' }));

describe('UnreadQuerySubscription', function UnreadQuerySubscriptionSpecs() {
  describe('the query', () => {
    beforeEach(() => {
      spyOn(QuerySubscription.prototype, 'update');
    });

    it('keeps a thread read in the view, inside the view categories, on the ThreadCategory subselect', () => {
      const subscription = new UnreadQuerySubscription(['inbox']);
      markRead();
      const sql = subscription._query.sql();
      expect(sql).toContain('WHERE `id` IN (SELECT `id` FROM `ThreadCategory`');
      expect(sql).toContain(
        "`ThreadCategory`.`value` IN ('inbox') AND ((`ThreadCategory`.`unread` != 0 AND `ThreadCategory`.`inAllMail` != 0) OR `ThreadCategory`.`id` = 't1')"
      );
      subscription.onLastCallbackRemoved();
    });

    it('drops a thread from the whitelist when it is moved', () => {
      const subscription = new UnreadQuerySubscription(['inbox']);
      markRead();
      Actions.queueTask(
        new ChangeFolderTask({
          threads: [thread],
          folder: new Folder({ id: 'archive', accountId: 'a1', path: 'Archive' }),
          source: 'spec',
        })
      );
      expect(subscription._query.sql()).not.toContain("'t1'");
      subscription.onLastCallbackRemoved();
    });

    it('starts empty, whatever earlier subscriptions whitelisted', () => {
      const earlier = new UnreadQuerySubscription(['inbox']);
      markRead();
      earlier.onLastCallbackRemoved();
      const next = new UnreadQuerySubscription(['inbox']);
      expect(next._query.sql()).not.toContain("'t1'");
      next.onLastCallbackRemoved();
    });
  });

  // The SQL text cases above hold for the SQLite path; these read through either transport.
  describe('reading', () =>
    forEachTransport((reads) => {
      const inbox = new Folder({ id: 'inbox', accountId: 'a1', path: 'INBOX' });
      const stored = (id: string, unread: boolean, ts: number) =>
        new Thread({
          id,
          accountId: 'a1',
          folders: [inbox],
          unread,
          inAllMail: true,
          lastMessageReceivedTimestamp: new Date(ts * 1000),
        });

      it('keeps a thread read in the view on the next full read', async () => {
        reads().add(stored('t1', true, 3000), stored('t2', true, 2000), stored('t3', false, 1000));
        const subscription = new UnreadQuerySubscription(['inbox']);
        const results = [];
        subscription.addCallback((set: any) => results.push(set.models().map((t) => t.id)));
        subscription.replaceRange({ start: 0, end: 50 });
        await settle();
        expect(results[results.length - 1]).toEqual(['t1', 't2']);

        markRead();
        const read = stored('t1', false, 3000);
        reads().add(read);
        subscription.applyChangeRecord(
          new DatabaseChangeRecord({
            type: 'persist',
            objectClass: 'Thread',
            objects: [read],
            objectsRawJSON: [read.toJSON()],
          })
        );
        await settle();
        expect(results[results.length - 1]).toEqual(['t1', 't2']);

        // A full read (after a reconnect) asks for the whitelisted thread too.
        subscription.rerun();
        await settle();
        expect(results[results.length - 1]).toEqual(['t1', 't2']);
        const last = reads().queries('Thread').pop();
        expect(JSON.stringify(last.where)).toContain('"t1"');
        subscription.onLastCallbackRemoved();
      });
    }));
});
