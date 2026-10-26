// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
What a thread list row shows of a thread's messages. On a live query the engine sends it with
the row (`thread.summary`, ThreadSummaryJson, crates/tern-store summary.rs); otherwise the data
source joins the thread's messages (`__messages`, hidden ones left out) and it is worked out
from them here the way the engine does.

Core row code reads only these, never `__messages`: on the summary route `__messages` is a
v1 compatibility getter that loads the messages when a plugin reads it (data-source.ts).
*/
import * as Utils from '../../data/models/utils';
import { Contact } from '../../data/models/contact';
import type { Message } from '../../data/models/message';
import type { Thread } from '../../data/models/thread';

type ListThread = Thread & { __messages?: Message[] };

export interface ParticipantToken {
  contact: Contact;
  unread: boolean;
}

function messagesOf(thread: ListThread): Message[] | null {
  return thread.__messages instanceof Array ? thread.__messages : null;
}

// Visible messages, drafts included.
export function listMessageCount(thread: ListThread): number {
  if (thread.summary) return thread.summary.messageCount;
  return messagesOf(thread)?.length || 0;
}

export function listHasDraft(thread: ListThread): boolean {
  if (thread.summary) return thread.summary.hasDraft;
  return !!(messagesOf(thread) || []).find((m) => m.draft);
}

// A message has a file that shows the attachment icon.
export function listHasAttachment(thread: ListThread): boolean {
  if (thread.summary) return thread.summary.hasAttachment;
  const messages = messagesOf(thread) || [];
  return (
    thread.attachmentCount > 0 && !!messages.find((m) => Utils.showIconForAttachments(m.files))
  );
}

// The last message's snippet that isn't empty; the thread's while it has no messages loaded.
export function listSnippet(thread: ListThread): string | null {
  if (thread.summary) {
    return thread.summary.messageCount > 0 ? thread.summary.snippet || null : thread.snippet;
  }
  const messages = messagesOf(thread) || [];
  if (messages.length === 0) {
    return thread.snippet;
  }
  for (let ii = messages.length - 1; ii >= 0; ii--) {
    if (messages[ii].snippet) return messages[ii].snippet;
  }
  return null;
}

// The thread has more than one sent or received message and the last is the user's.
export function listLastReply(thread: ListThread): 'replied' | 'forwarded' | null {
  if (thread.summary) return thread.summary.lastReply || null;
  const msgs = (messagesOf(thread) || []).filter((m) => m.id && !m.draft);
  const last = msgs[msgs.length - 1];
  if (msgs.length > 1 && (last.from[0] != null ? last.from[0].isMe() : undefined)) {
    return last.isForwarded() ? 'forwarded' : 'replied';
  }
  return null;
}

/*
The senders in date order, a sender repeating the one before folded into it (their
recipients when every message is the user's); null when the row has no messages to say, so
the thread's participants stand in.
*/
export function listSenders(thread: ListThread): ParticipantToken[] | null {
  if (thread.summary) {
    return thread.summary.senders.map(({ email, name, unread }) => ({
      contact: new Contact({ email, name: name || undefined, accountId: thread.accountId }),
      unread,
    }));
  }
  const messages = messagesOf(thread);
  if (!messages) return null;
  const tokens: ParticipantToken[] = [];

  let field: 'from' | 'to' = 'from';
  if (messages.every((message) => message.isFromMe())) {
    field = 'to';
  }

  for (const message of messages) {
    if (message.draft) {
      continue;
    }

    for (const contact of message[field]) {
      if (tokens.length === 0) {
        tokens.push({ contact, unread: message.unread });
      } else {
        const lastToken = tokens[tokens.length - 1];
        const lastContact = lastToken.contact;

        const sameEmail = Utils.emailIsEquivalent(lastContact.email, contact.email);
        const sameName = lastContact.name === contact.name;
        if (sameEmail && sameName) {
          if (!lastToken.unread) {
            lastToken.unread = message.unread;
          }
        } else {
          tokens.push({ contact, unread: message.unread });
        }
      }
    }
  }

  return tokens;
}
