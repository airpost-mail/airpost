import React from 'react';
import AccountColorBar from '../../components/account-color-bar';
import { ThreadWithMessagesMetadata } from './types';
import { listMessageCount, listSenders, ParticipantToken } from './summary';

type ParticipantItem = ParticipantToken | { spacer: true };

function getTokensFromParticipants(thread: ThreadWithMessagesMetadata) {
  let contacts = thread.participants != null ? thread.participants : [];
  contacts = contacts.filter((contact) => !contact.isMe());
  return contacts.map((contact) => ({ contact, unread: false }));
}

export function getTokens(thread: ThreadWithMessagesMetadata) {
  let list: ParticipantItem[] = listSenders(thread) || getTokensFromParticipants(thread);

  // If no participants, we should at least add current user as sole participant
  if (list.length === 0 && thread.participants != null && thread.participants.length > 0) {
    list.push({ contact: thread.participants[0], unread: false });
  }

  // We only ever want to show three. Ben...Kevin... Marty
  // But we want the *right* three.
  if (list.length > 3) {
    const listTrimmed: ParticipantItem[] = [
      // Always include the first item
      list[0],
      { spacer: true },

      // Always include last two items
      list[list.length - 2],
      list[list.length - 1],
    ];
    list = listTrimmed;
  }

  return list;
}

function renderSpans(thread: ThreadWithMessagesMetadata, items: ParticipantItem[]) {
  const spans: JSX.Element[] = [];
  let accumulated: string | null = null;
  let accumulatedUnread: boolean | undefined = false;

  const flush = function () {
    if (accumulated) {
      spans.push(
        <span key={spans.length} className={`unread-${accumulatedUnread}`}>
          {accumulated}
        </span>
      );
    }
    accumulated = null;
    accumulatedUnread = false;
  };

  const accumulate = function (text: string, unread?: boolean) {
    if (accumulated && unread && accumulatedUnread !== unread) {
      flush();
    }
    if (accumulated) {
      accumulated += text;
    } else {
      accumulated = text;
      accumulatedUnread = unread;
    }
  };

  for (let idx = 0; idx < items.length; idx++) {
    const item = items[idx];
    if ('spacer' in item) {
      accumulate('...');
    } else {
      const { contact, unread } = item;
      let short = contact.email;
      if (contact.name && contact.name.length > 0) {
        if (items.length > 1) {
          short = contact.displayName({
            includeAccountLabel: false,
            compact: !AppEnv.config.get('core.reading.detailedNames'),
          });
        } else {
          short = contact.displayName({ includeAccountLabel: false });
        }
      }
      if (idx < items.length - 1 && !('spacer' in items[idx + 1])) {
        short += ', ';
      }
      accumulate(short, unread);
    }
  }

  if (!thread.summary && !thread.__messages) {
    throw new Error('ThreadListParticipants requires a summary or __messages.');
  }

  const count = listMessageCount(thread);
  if (count > 1) {
    accumulate(` (${count})`);
  }

  flush();

  return spans;
}

const ThreadListParticipants: React.FC<{ thread: ThreadWithMessagesMetadata }> = React.memo(
  ({ thread }) => (
    <div className="participants" dir="auto">
      <AccountColorBar accountId={thread.accountId} />
      {renderSpans(thread, getTokens(thread))}
    </div>
  ),
  (prev, next) => prev.thread === next.thread
);
ThreadListParticipants.displayName = 'ThreadListParticipants';

export default ThreadListParticipants;
