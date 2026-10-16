import type { MessageJSON } from '@/engine';
import { openComposer, type ComposeMode } from '@/features/composer';

/** A draft answering another message, edited inline in its thread (not a standalone one). */
export function isReplyDraft(m: MessageJSON): boolean {
  return m.draft && !!(m.rthMsgId || m.fwdMsgId);
}

/** Opens the composer for `m`; on a draft every action means "keep editing it". */
export function compose(mode: Exclude<ComposeMode, 'new' | 'edit-draft'>, m: MessageJSON) {
  if (m.draft) {
    openComposer({
      mode: 'edit-draft',
      accountId: m.aid,
      threadId: m.threadId,
      draftId: m.id,
      placement: isReplyDraft(m) ? 'inline' : 'docked',
    });
  } else {
    openComposer({ mode, accountId: m.aid, threadId: m.threadId, messageId: m.id });
  }
}

/** The message r / a / f act on: the newest that isn't a draft. */
export function replyTarget(messages: MessageJSON[]): MessageJSON | undefined {
  for (let i = messages.length - 1; i >= 0; i--) if (!messages[i].draft) return messages[i];
  return messages[messages.length - 1];
}

/**
 * The reply draft to show inline when a thread opens: its newest one, unless a composer
 * already has it or it was closed inline this session.
 */
export function draftToResume(
  messages: MessageJSON[],
  taken: (draftId: string) => boolean,
): MessageJSON | undefined {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (isReplyDraft(m)) return taken(m.id) ? undefined : m;
  }
  return undefined;
}
