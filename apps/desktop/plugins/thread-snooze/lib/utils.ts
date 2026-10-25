import moment, { Moment } from 'moment';
import {
  localized,
  Actions,
  Label,
  DateUtils,
  TaskFactory,
  CategoryStore,
  ChangeLabelsTask,
  ChangeFolderTask,
  Thread,
} from 'mailspring-exports';

export function snoozedUntilMessage(snoozeDate: string | null, now: Moment = moment()) {
  let message = localized('Snoozed');
  if (snoozeDate) {
    const dateFormat = DateUtils.DATE_FORMAT_SHORT;
    const date = moment(snoozeDate);
    const hourDifference = moment.duration(date.diff(now)).asHours();

    message += ` ${DateUtils.format(date, dateFormat)}`;
  }
  return message;
}

export function moveThreads(
  threads: Thread[],
  { snooze, description }: { snooze?: boolean; description?: string } = {}
) {
  const tasks = TaskFactory.tasksForThreadsByAccountId(threads, (accountThreads, accountId) => {
    const snoozeCat = CategoryStore.getCategoryByRole(accountId, 'snoozed');
    const inboxCat = CategoryStore.getInboxCategory(accountId);

    if (snoozeCat instanceof Label) {
      return new ChangeLabelsTask({
        source: 'Snooze Move',
        threads: accountThreads,
        taskDescription: description,
        labelsToAdd: snooze ? [snoozeCat] : [inboxCat],
        labelsToRemove: snooze ? [inboxCat] : [snoozeCat],
        canBeUndone: snooze ? true : false,
      });
    }
    // Snoozing moves the Inbox copy only, and unsnoozing brings back only what was
    // snoozed; a Sent copy of the same message is never dragged along.
    const sourceCat = snooze ? inboxCat : snoozeCat;
    return new ChangeFolderTask({
      source: 'Snooze Move',
      threads: accountThreads,
      taskDescription: description,
      folder: snooze ? snoozeCat : inboxCat,
      sourceFolderIds: sourceCat ? [sourceCat.id] : [],
      canBeUndone: snooze ? true : false,
    });
  });

  Actions.queueTasks(tasks);
}
