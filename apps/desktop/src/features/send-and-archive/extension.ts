import * as Actions from '../../data/actions';
import { Thread } from '../../data/models/thread';
import { Message } from '../../data/models/message';
import DatabaseStore from '../../data/database/store';
import { TaskFactory } from '../../data/tasks/factory';
import { SendDraftTask } from '../../data/tasks/send-draft';

export const name = 'SendAndArchiveExtension';

export function sendActions() {
  return [
    {
      title: 'Send and Archive',
      iconUrl: 'tern-asset://send-and-archive/images/composer-archive@2x.png',
      isAvailableForDraft({ draft }: { draft: Message }) {
        return draft.threadId;
      },
      async performSendAction({ draft }: { draft: Message }) {
        Actions.queueTask(SendDraftTask.forSending(draft));

        if (!draft.threadId) return;

        const threads = await DatabaseStore.modelify<Thread>(Thread, [draft.threadId]);
        const tasks = TaskFactory.tasksForArchiving({
          source: 'Send and Archive',
          threads: threads,
        });
        Actions.queueTasks(tasks);
      },
    },
  ];
}
