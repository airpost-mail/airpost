/* eslint quote-props: 0 */
import { Thread } from '../../../src/flux/models/thread';
import TestModel from '../../fixtures/db-test-model';
import ModelQuery from '../../../src/flux/models/query';
import DatabaseStore from '../../../src/flux/stores/database-store';
import { forEachTransport } from '../../mock-engine';

const testMatchers = { id: 'b' };

describe('DatabaseStore', function DatabaseStoreSpecs() {
  beforeEach(() => {
    (TestModel as any).configureBasic();
    spyOn(ModelQuery.prototype, 'where').andCallThrough();

    this.performed = [];

    // Note: We spy on _query and test all of the convenience methods that sit above
    // it. None of these tests evaluate whether _query works!
    jasmine.unspy(DatabaseStore, '_query');
    spyOn(DatabaseStore, '_query').andCallFake((query, values = []) => {
      this.performed.push({ query, values });
      return Promise.resolve([]);
    });
  });

  describe('find', () =>
    it('should return a ModelQuery for retrieving a single item by Id', () => {
      const q = DatabaseStore.find(TestModel, '4');
      expect(q.sql()).toBe(
        "SELECT `TestModel`.`data` FROM `TestModel`  WHERE `TestModel`.`id` = '4'  LIMIT 1"
      );
    }));

  describe('findBy', () => {
    it('should pass the provided predicates on to the ModelQuery', () => {
      DatabaseStore.findBy<TestModel>(TestModel, testMatchers);
      expect(ModelQuery.prototype.where).toHaveBeenCalledWith(testMatchers);
    });

    it('should return a ModelQuery ready to be executed', () => {
      const q = DatabaseStore.findBy<TestModel>(TestModel, testMatchers);
      expect(q.sql()).toBe(
        "SELECT `TestModel`.`data` FROM `TestModel`  WHERE `TestModel`.`id` = 'b'  LIMIT 1"
      );
    });
  });

  describe('findAll', () => {
    it('should pass the provided predicates on to the ModelQuery', () => {
      DatabaseStore.findAll<TestModel>(TestModel, testMatchers);
      expect(ModelQuery.prototype.where).toHaveBeenCalledWith(testMatchers);
    });

    it('should return a ModelQuery ready to be executed', () => {
      const q = DatabaseStore.findAll<TestModel>(TestModel, testMatchers);
      expect(q.sql()).toBe(
        "SELECT `TestModel`.`data` FROM `TestModel`  WHERE `TestModel`.`id` = 'b'  "
      );
    });
  });

  // modelify reads the ids it was given, so it runs on both transports.
  describe('modelify', () =>
    forEachTransport((reads) => {
      const ids = (models: Thread[]) => models.map((m) => m.id);

      beforeEach(() => {
        this.models = ['A', 'B', 'C', 'D', 'E', 'F', 'G'].map(
          (l) => new Thread({ id: `local-${l}`, accountId: TEST_ACCOUNT_ID })
        );
        reads().add(...this.models);
      });

      describe('when given an array or input that is not an array', () =>
        it('resolves immediately with an empty array', async () => {
          expect(await DatabaseStore.modelify<Thread>(Thread, null)).toEqual([]);
          expect(reads().queries('Thread')).toEqual([]);
        }));

      describe('when given an array of mixed IDs, and models', () =>
        it('resolves with an array of models', async () => {
          const input = ['local-F', 'local-B', 'local-C', 'local-D', this.models[6]];
          const output = await DatabaseStore.modelify<Thread>(Thread, input);
          expect(ids(output)).toEqual(['local-F', 'local-B', 'local-C', 'local-D', 'local-G']);
          // A model passed in is kept as it is; only the ids are read.
          expect(output[4]).toBe(this.models[6]);
          expect(output[0] instanceof Thread).toBe(true);
          expect(reads().queries('Thread').length).toBe(1);
        }));

      describe('when the input is only IDs', () =>
        it('resolves with an array of models', async () => {
          const output = await DatabaseStore.modelify<Thread>(Thread, [
            'local-D',
            'local-F',
            'local-G',
          ]);
          expect(ids(output)).toEqual(['local-D', 'local-F', 'local-G']);
        }));

      describe('when the input is all models', () =>
        it('resolves with an array of models', async () => {
          const input = [this.models[0], this.models[1], this.models[2], this.models[3]];
          const output = await DatabaseStore.modelify<Thread>(Thread, input);
          expect(output).toEqual(input);
          expect(reads().queries('Thread')).toEqual([]);
        }));

      describe('when the input contains IDs not present in the database', () =>
        it('omits missing entries rather than returning undefined', async () => {
          const output = await DatabaseStore.modelify<Thread>(Thread, [
            'local-A',
            'does-not-exist',
            'local-C',
          ]);
          expect(ids(output)).toEqual(['local-A', 'local-C']);
        }));
    }));

  describe('count', () => {
    it('should pass the provided predicates on to the ModelQuery', () => {
      DatabaseStore.findAll<TestModel>(TestModel, testMatchers);
      expect(ModelQuery.prototype.where).toHaveBeenCalledWith(testMatchers);
    });

    it('should return a ModelQuery configured for COUNT ready to be executed', () => {
      const q = DatabaseStore.findAll<TestModel>(TestModel, testMatchers);
      expect(q.sql()).toBe(
        "SELECT `TestModel`.`data` FROM `TestModel`  WHERE `TestModel`.`id` = 'b'  "
      );
    });
  });
});
