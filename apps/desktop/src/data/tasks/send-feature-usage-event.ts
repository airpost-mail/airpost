import { Task } from './task';
import * as Attributes from '../attributes';
import { AccountStore } from '../stores/account';
import { AttributeValues } from '../models/model';

/**
 * Nothing queues this task. The class stays registered so task rows already
 * persisted by older builds still inflate when the engine streams them back.
 */
export class SendFeatureUsageEventTask extends Task {
  static attributes = {
    ...Task.attributes,

    feature: Attributes.String({
      modelKey: 'feature',
    }),
  };

  feature: string;

  constructor(data: AttributeValues<typeof SendFeatureUsageEventTask.attributes> = {}) {
    super(data);

    // Tasks must have an accountId so they can be assigned to a sync worker.
    // We don't really care what sync worker handles this, since it's just a
    // POST to id.getmailspring.com. Just assign the first account ID.
    if (!this.accountId) {
      this.accountId = AccountStore.accountIds()[0];
    }
  }
}
