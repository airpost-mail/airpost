// `include: ["summary"]` on Thread rows, as crates/tern-store/src/summary.rs builds it from
// the thread's visible messages.
import type { ContactRef, MessageJSON, ThreadJSON } from '../types';
import type { ThreadSender, ThreadSummaryJson } from '@tern/client';
import type { MockStore } from './store';

/** Inline images up to this size don't count as attachments. */
const INLINE_IMAGE_MAX = 12 * 1024;

function hasAttachment(m: MessageJSON): boolean {
  return (m.files ?? []).some(
    (f) => !(f.contentId && f.contentType.startsWith('image/') && f.size <= INLINE_IMAGE_MAX),
  );
}

export function threadSummary(s: MockStore, t: ThreadJSON): ThreadSummaryJson {
  const own = s.ownEmails(t.aid);
  const isMe = (c: ContactRef) => own.has(c.email.toLowerCase());
  const visible = s.threadMessages(t.id).filter((m) => !m.hidden);
  const mail = visible.filter((m) => !m.draft);
  const fromMe = (m: MessageJSON) => m.from.some(isMe);

  const senders: ThreadSender[] = [];
  const allMine = mail.length > 0 && mail.every(fromMe);
  for (const m of mail) {
    const people = allMine ? [...m.to, ...m.cc] : m.from.slice(0, 1);
    for (const c of people) {
      const last = senders[senders.length - 1];
      if (last && last.email.toLowerCase() === c.email.toLowerCase()) {
        last.unread ||= m.unread;
        continue;
      }
      senders.push({ email: c.email, name: c.name, me: isMe(c), unread: m.unread });
    }
  }

  const last = mail[mail.length - 1];
  const lastReply =
    mail.length > 1 && last && fromMe(last)
      ? /^\s*(fwd?|fw)\s*:/i.test(last.subject ?? '')
        ? 'forwarded'
        : 'replied'
      : undefined;
  const sending = [...s.tasks.values()].some(
    (task) =>
      task.__cls === 'SendDraftTask' &&
      task.status !== 'complete' &&
      task.status !== 'cancelled' &&
      visible.some((m) => m.draft && m.hMsgId === task.headerMessageId),
  );
  const snippet = [...visible].reverse().find((m) => m.snippet)?.snippet ?? null;

  return {
    messageCount: visible.length,
    senders,
    hasDraft: visible.some((m) => m.draft),
    sending,
    hasAttachment: visible.some(hasAttachment),
    snippet,
    ...(lastReply ? { lastReply } : {}),
  };
}
