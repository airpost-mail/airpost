import QuerySubscriptionPool from '../../src/flux/models/query-subscription-pool';
import DatabaseStore from '../../src/flux/stores/database-store';
import { Label } from '../../src/flux/models/label';
import { forEachTransport, settle } from '../mock-engine';

describe('QuerySubscriptionPool', function QuerySubscriptionPoolSpecs() {
  forEachTransport((reads) => {
    beforeEach(() => {
      this.query = DatabaseStore.findAll<Label>(Label);
      this.queryKey = JSON.stringify(this.query.toJSON());
      QuerySubscriptionPool._subscriptions = {};
      QuerySubscriptionPool._cleanupChecks = [];
    });

    describe('add', () => {
      it('should deliver the first read to the callback', async () => {
        reads().add(new Label({ id: 'l1', accountId: TEST_ACCOUNT_ID, path: 'Work' }));
        const callback = jasmine.createSpy('callback');
        const unsub = QuerySubscriptionPool.add(this.query, callback);
        await settle();
        expect(callback).toHaveBeenCalled();
        expect(callback.mostRecentCall.args[0].map((l) => l.id)).toEqual(['l1']);
        expect(reads().queries('Label').length).toBe(1);
        unsub();
        advanceClock();
      });

      it('should add a new subscription with the callback', () => {
        const callback = jasmine.createSpy('callback');
        QuerySubscriptionPool.add(this.query, callback);
        expect(QuerySubscriptionPool._subscriptions[this.queryKey]).toBeDefined();

        const subscription = QuerySubscriptionPool._subscriptions[this.queryKey];
        expect(subscription.hasCallback(callback)).toBe(true);
      });

      it('should yield database changes to the subscription', () => {
        const callback = jasmine.createSpy('callback');
        QuerySubscriptionPool.add(this.query, callback);
        const subscription = QuerySubscriptionPool._subscriptions[this.queryKey];
        spyOn(subscription, 'applyChangeRecord');

        const record = { objectType: 'whateves' } as any;
        QuerySubscriptionPool._onChange(record);
        expect(subscription.applyChangeRecord).toHaveBeenCalledWith(record);
      });

      describe('unsubscribe', () => {
        it('should return an unsubscribe method', () => {
          expect(QuerySubscriptionPool.add(this.query, () => {}) instanceof Function).toBe(true);
        });

        it('should remove the callback from the subscription', () => {
          const cb = () => {};

          const unsub = QuerySubscriptionPool.add(this.query, cb);
          const subscription = QuerySubscriptionPool._subscriptions[this.queryKey];

          expect(subscription.hasCallback(cb)).toBe(true);
          unsub();
          expect(subscription.hasCallback(cb)).toBe(false);
        });

        it("should wait before removing th subscription to make sure it's not reused", () => {
          const unsub = QuerySubscriptionPool.add(this.query, () => {});
          expect(QuerySubscriptionPool._subscriptions[this.queryKey]).toBeDefined();
          unsub();
          expect(QuerySubscriptionPool._subscriptions[this.queryKey]).toBeDefined();
          advanceClock();
          expect(QuerySubscriptionPool._subscriptions[this.queryKey]).toBeUndefined();
        });
      });
    });
  });
});
