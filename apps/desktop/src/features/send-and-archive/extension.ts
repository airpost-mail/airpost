import * as Actions from '../../data/actions';
import { Message } from '../../data/models/message';

export const name = 'SendAndArchiveExtension';

// The engine performs it: draft.send {after: "send-and-archive"} archives the thread once the
// message is sent (DraftStore._onSendDraft).
export function sendActions() {
  return [
    {
      title: 'Send and Archive',
      iconUrl: 'tern-asset://send-and-archive/images/composer-archive@2x.png',
      isAvailableForDraft({ draft }: { draft: Message }) {
        return draft.threadId;
      },
      performSendAction({ draft }: { draft: Message }) {
        Actions.sendDraft(draft.headerMessageId, { actionKey: 'send-and-archive', delay: 0 });
      },
    },
  ];
}
