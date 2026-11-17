import TaskQueue from '../../../src/data/stores/task-queue';
import QuerySubscriptionPool from '../../../src/data/query/subscription-pool';
import { unmountAll } from './react-test-utils-extensions';

class MasterAfterEach {
  setup(loadSettings, afterEach) {
    const styleElementsToRestore = AppEnv.styles.getSnapshot();

    const self = this;
    afterEach(async function masterAfterEach() {
      // await destroyTestDatabase() TODO BEN
      AppEnv.packages.deactivatePackages();
      AppEnv.menu.template = [];

      if (!window.debugContent) {
        document.getElementById('jasmine-content').innerHTML = '';
      }
      unmountAll();
      // The pool drops subscriptions without callbacks on a timeout, which the fake clock's
      // reset between specs throws away; a later spec would get the stale one back.
      QuerySubscriptionPool._runCleanupChecks();

      jasmine.unspy(AppEnv, 'saveWindowState');

      // Spies first: a spec that stubs AppEnv.styles.addStyleSheet would otherwise swallow the
      // restored sheets and leave <managed-styles> empty for the specs after it.
      this.removeAllSpies();
      AppEnv.styles.restoreSnapshot(styleElementsToRestore);

      TaskQueue._onQueueChangedDebounced.cancel();
      if (TaskQueue._queue.length > 0) {
        console.inspect(TaskQueue._queue);
        TaskQueue._queue = [];
        throw new Error('Your test forgot to clean up the TaskQueue');
      }
      waits(0);
    }); // yield to ui thread to make screen update more frequently
  }
}

export default new MasterAfterEach();
