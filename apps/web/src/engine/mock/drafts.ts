// `draft.create` and `draft.validate` for the mock: the reply, forward and new drafts the
// web composer built before the engine did (airpost-compose builds them for the real one), and
// a few of airpost-compose's send checks.
import { format } from 'date-fns';
import { escapeHtml, sanitizeQuotedHtml, textToHtml } from '@/features/composer/html';
import { identitiesFor, ownEmails } from '@/features/composer/identities';
import {
  dedupeContacts,
  emailKey,
  formatContact,
  parseAddress,
} from '@/features/composer/recipients';
import { generateHeaderMessageId, generateId, nowSeconds } from '../model';
import { RpcError, RpcErrorCode, type DraftProblem, type RpcMethods } from '../rpc';
import type { AccountJSON, ContactRef, MessageJSON } from '../types';
import type { MockStore } from './store';

type CreateParams = RpcMethods['draft.create']['params'];

export function subjectWithPrefix(subject: string, prefix: 'Re:' | 'Fwd:'): string {
  if (/^fwd:/i.test(subject)) return subject.replace(/^fwd:/i, prefix);
  if (/^re:/i.test(subject)) return subject.replace(/^re:/i, prefix);
  return `${prefix} ${subject}`.trim();
}

/** The account's default alias, else its own address (the desktop's `Account.defaultMe`). */
function defaultFrom(account: AccountJSON): ContactRef {
  const alias = account.defaultAlias ? parseAddress(account.defaultAlias) : null;
  return alias
    ? { email: alias.email, name: alias.name ?? account.name }
    : { name: account.name, email: account.emailAddress };
}

/** For a reply: the alias the original was addressed to, else the account default. */
function replyFrom(account: AccountJSON, message: MessageJSON): ContactRef {
  const fallback = defaultFrom(account);
  const recipients = new Set([...message.to, ...message.cc].map((c) => emailKey(c.email)));
  const match = identitiesFor(account).find(
    (i) =>
      recipients.has(emailKey(i.contact.email)) &&
      emailKey(i.contact.email) !== emailKey(fallback.email),
  );
  return match?.contact ?? fallback;
}

export function participantsForReply(message: MessageJSON, own: Set<string>, all: boolean) {
  const isMe = (c: ContactRef) => own.has(emailKey(c.email));
  const useReplyTo = message.replyTo.length > 0 && !isMe(message.replyTo[0]);
  const fromMe = !useReplyTo && message.from.some(isMe);
  const to = dedupeContacts(useReplyTo ? message.replyTo : fromMe ? message.to : message.from);
  if (!all) return { to, cc: [] as ContactRef[] };
  const excluded = [...own, ...message.from.map((c) => c.email), ...to.map((c) => c.email)];
  const ccSource = fromMe ? message.cc : [...message.to, ...message.cc];
  return { to, cc: dedupeContacts(ccSource, excluded) };
}

const when = (m: MessageJSON) => format(new Date(m.date * 1000), "MMM d yyyy, 'at' h:mm a");

function quotable(m: MessageJSON): string {
  const body = m.body ?? '';
  return m.plaintext ? textToHtml(body) : sanitizeQuotedHtml(body);
}

function replyQuote(m: MessageJSON): string {
  const from = m.from[0] ?? { email: 'unknown' };
  return [
    `<div class="gmail_quote_attribution">${escapeHtml(`On ${when(m)}, ${formatContact(from)} wrote:`)}</div>`,
    '<blockquote class="gmail_quote" style="margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex;">',
    quotable(m),
    '<br></blockquote>',
  ].join('\n');
}

function forwardQuote(m: MessageJSON): string {
  const list = (cs: ContactRef[]) => escapeHtml(cs.map(formatContact).join(', '));
  const fields: string[] = [];
  if (m.from.length) fields.push(`From: ${list(m.from)}`);
  fields.push(`Subject: ${escapeHtml(m.subject)}`);
  fields.push(`Date: ${escapeHtml(when(m))}`);
  if (m.to.length) fields.push(`To: ${list(m.to)}`);
  if (m.cc.length) fields.push(`Cc: ${list(m.cc)}`);
  return [
    '<div class="gmail_quote">',
    '---------- Forwarded Message ---------<br><br>',
    fields.join('<br>'),
    '<br><br>',
    quotable(m),
    '<br></div>',
  ].join('\n');
}

/** The account's automatic Cc/Bcc addresses. */
function autoaddress(account: AccountJSON): { cc: ContactRef[]; bcc: ContactRef[] } {
  const list = (account.autoaddress?.value ?? '')
    .split(/[,;]/)
    .map((e) => e.trim())
    .filter(Boolean)
    .map((email) => ({ email }));
  return account.autoaddress?.type === 'cc' ? { cc: list, bcc: [] } : { cc: [], bcc: list };
}

export function createDraft(s: MockStore, p: CreateParams, account: AccountJSON): MessageJSON {
  const from = defaultFrom(account);
  const hMsgId = generateHeaderMessageId(from.email.split('@')[1]);
  const fields = (p.fields ?? {}) as Partial<Pick<MessageJSON, 'to' | 'cc' | 'bcc' | 'subject'>> & {
    body?: string;
  };
  const auto = autoaddress(account);
  const draft: MessageJSON = {
    id: generateId(),
    aid: account.id,
    v: 0,
    __cls: 'Message',
    subject: fields.subject ?? '',
    date: nowSeconds(),
    threadId: '',
    hMsgId,
    rthMsgId: null,
    fwdMsgId: null,
    unread: false,
    starred: false,
    draft: true,
    plaintext: false,
    body: fields.body ?? '',
    from: [from],
    to: fields.to ?? [],
    cc: dedupeContacts([...(fields.cc ?? []), ...auto.cc]),
    bcc: dedupeContacts([...(fields.bcc ?? []), ...auto.bcc]),
    replyTo: [],
    files: [],
    metadata: [],
  };
  if (p.kind === 'new') return draft;

  const stored = p.messageId ? s.messages.get(p.messageId) : undefined;
  if (!stored || stored.aid !== account.id) {
    throw new RpcError({ code: RpcErrorCode.NotFound, message: `no message ${p.messageId}` });
  }
  const m = s.withBody(stored);
  if (m.body == null) {
    throw new RpcError({
      code: RpcErrorCode.NotFound,
      message: `message ${m.id} isn't downloaded yet`,
    });
  }
  draft.threadId = m.threadId;
  if (p.kind === 'forward') {
    return {
      ...draft,
      subject: subjectWithPrefix(m.subject, 'Fwd:'),
      fwdMsgId: m.hMsgId,
      files: m.files.filter((f) => !f.contentId).map((f) => ({ ...f })),
      body: `<br><br>${forwardQuote(m)}`,
    };
  }
  const own = ownEmails([...s.accounts.values()]);
  const { to, cc } = participantsForReply(m, own, p.kind === 'reply-all');
  return {
    ...draft,
    from: [replyFrom(account, m)],
    to,
    cc: dedupeContacts([...cc, ...draft.cc]),
    subject: subjectWithPrefix(m.subject, 'Re:'),
    rthMsgId: m.hMsgId,
    body: `<br><br>${replyQuote(m)}`,
  };
}

const ADDRESS = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** The saved draft's no-recipients, invalid-recipient and blank-subject checks. */
export function validateDraft(
  s: MockStore,
  p: RpcMethods['draft.validate']['params'],
): RpcMethods['draft.validate']['result'] {
  const d = findDraft(s, p.accountId, p.headerMessageId);
  const errors: DraftProblem[] = [];
  const warnings: DraftProblem[] = [];
  const all = [...d.to, ...d.cc, ...d.bcc];
  if (all.length === 0) {
    errors.push({
      code: 'no-recipients',
      message: 'You need to provide one or more recipients before sending the message.',
      group: 'recipients',
    });
  }
  const bad = all.find((c) => !ADDRESS.test(c.email));
  if (bad) {
    errors.push({
      code: 'invalid-recipient',
      message: `${bad.email} is not a valid email address - please remove or edit it before sending.`,
      group: 'recipients',
    });
  }
  if (!d.subject.trim()) {
    warnings.push({ code: 'blank-subject', message: 'The subject field is blank.', group: 'misc' });
  }
  return { errors, warnings };
}

export function findDraft(s: MockStore, accountId: string, headerMessageId: string): MessageJSON {
  const d = [...s.messages.values()].find(
    (m) => m.draft && m.aid === accountId && m.hMsgId === headerMessageId,
  );
  if (!d) {
    throw new RpcError({ code: RpcErrorCode.NotFound, message: `no draft ${headerMessageId}` });
  }
  return d;
}
