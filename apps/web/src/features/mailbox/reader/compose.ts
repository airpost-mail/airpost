import type { MessageJSON } from '@/engine';
import { openComposer, type ComposeMode } from '@/features/composer';

/** Opens the composer for `m`; on a draft every action means "keep editing it". */
export function compose(mode: Exclude<ComposeMode, 'new' | 'edit-draft'>, m: MessageJSON) {
  if (m.draft) {
    openComposer({ mode: 'edit-draft', accountId: m.aid, threadId: m.threadId, draftId: m.id });
  } else {
    openComposer({ mode, accountId: m.aid, threadId: m.threadId, messageId: m.id });
  }
}

/** The message r / a / f act on: the newest that isn't a draft. */
export function replyTarget(messages: MessageJSON[]): MessageJSON | undefined {
  for (let i = messages.length - 1; i >= 0; i--) if (!messages[i].draft) return messages[i];
  return messages[messages.length - 1];
}
