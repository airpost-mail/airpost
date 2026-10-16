// The composer's working copy of a draft and how it maps to the engine's Message JSON.
// Reply/forward construction follows the desktop's DraftFactory and Message models.
import { format } from 'date-fns';
import {
  contactDisplayName,
  generateHeaderMessageId,
  generateId,
  nowSeconds,
  type AccountJSON,
  type ContactRef,
  type FileJSON,
  type MessageJSON,
  type PluginMetadataJSON,
} from '@/engine';
import { escapeHtml, sanitizeQuotedHtml, splitQuoted, textToHtml } from './html';
import type { Identity } from './identities';
import { dedupeContacts, emailKey, formatContact } from './recipients';

export type ReplyMode = 'reply' | 'reply-all';

export interface DraftAttachment {
  file: FileJSON;
  /** Bytes picked in this browser that the engine does not have yet. */
  local?: File;
}

export interface DraftState {
  id: string;
  headerMessageId: string;
  accountId: string;
  from: ContactRef;
  to: ContactRef[];
  cc: ContactRef[];
  bcc: ContactRef[];
  subject: string;
  /** The editor's HTML: what the user wrote, without the quoted original. */
  body: string;
  /** Quoted original (reply/forward), appended to `body` when saving. */
  quoted: string | null;
  /** The quoted original's body has not arrived from the engine yet. */
  awaitingQuote: boolean;
  includeQuoted: boolean;
  threadId?: string;
  replyToHeaderMessageId?: string;
  forwardedHeaderMessageId?: string;
  files: DraftAttachment[];
  metadata: PluginMetadataJSON[];
}

function base(identity: Identity): DraftState {
  return {
    id: generateId(),
    headerMessageId: generateHeaderMessageId(identity.contact.email.split('@')[1]),
    accountId: identity.accountId,
    from: identity.contact,
    to: [],
    cc: [],
    bcc: [],
    subject: '',
    body: '',
    quoted: null,
    awaitingQuote: false,
    includeQuoted: true,
    files: [],
    metadata: [],
  };
}

export function subjectWithPrefix(subject: string, prefix: 'Re:' | 'Fwd:'): string {
  if (/^fwd:/i.test(subject)) return subject.replace(/^fwd:/i, prefix);
  if (/^re:/i.test(subject)) return subject.replace(/^re:/i, prefix);
  return `${prefix} ${subject}`.trim();
}

/** The account's automatic Cc/Bcc address, if set. */
function autoaddress(account: AccountJSON | undefined) {
  const value = account?.autoaddress?.value?.trim();
  if (!value) return {};
  const list = value
    .split(/[,;]/)
    .map((e) => e.trim())
    .filter(Boolean)
    .map((email) => ({ email }));
  return account!.autoaddress.type === 'cc' ? { cc: list } : { bcc: list };
}

export function newDraft(
  identity: Identity,
  fields: {
    to?: ContactRef[];
    cc?: ContactRef[];
    bcc?: ContactRef[];
    subject?: string;
    body?: string;
  } = {},
  account?: AccountJSON,
): DraftState {
  const auto = autoaddress(account);
  return {
    ...base(identity),
    to: fields.to ?? [],
    cc: dedupeContacts([...(fields.cc ?? []), ...(auto.cc ?? [])]),
    bcc: dedupeContacts([...(fields.bcc ?? []), ...(auto.bcc ?? [])]),
    subject: fields.subject ?? '',
    body: fields.body ? textToHtml(fields.body) : '',
  };
}

function isMe(c: ContactRef, own: Set<string>) {
  return own.has(emailKey(c.email));
}

export function participantsForReply(message: MessageJSON, own: Set<string>, mode: ReplyMode) {
  const useReplyTo = message.replyTo.length > 0 && !isMe(message.replyTo[0], own);
  const fromMe = !useReplyTo && message.from.some((c) => isMe(c, own));
  const to = dedupeContacts(useReplyTo ? message.replyTo : fromMe ? message.to : message.from);
  if (mode === 'reply') return { to, cc: [] as ContactRef[] };

  const excluded = [...own, ...message.from.map((c) => c.email), ...to.map((c) => c.email)];
  const ccSource = fromMe ? message.cc : [...message.to, ...message.cc];
  return { to, cc: dedupeContacts(ccSource, excluded) };
}

export function replyAttribution(message: MessageJSON): string {
  const from = message.from[0] ?? { email: 'unknown' };
  const date = format(new Date(message.date * 1000), "MMM d yyyy, 'at' h:mm a");
  return `On ${date}, ${formatContact(from)} wrote:`;
}

function quotableBody(message: MessageJSON): string {
  const body = message.body ?? '';
  return message.plaintext ? textToHtml(body) : sanitizeQuotedHtml(body);
}

export function quoteForReply(message: MessageJSON): string {
  return [
    `<div class="gmail_quote_attribution">${escapeHtml(replyAttribution(message))}</div>`,
    '<blockquote class="gmail_quote" style="margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex;">',
    quotableBody(message),
    '<br></blockquote>',
  ].join('\n');
}

export function quoteForForward(message: MessageJSON): string {
  const list = (cs: ContactRef[]) => escapeHtml(cs.map(formatContact).join(', '));
  const fields: string[] = [];
  if (message.from.length) fields.push(`From: ${list(message.from)}`);
  fields.push(`Subject: ${escapeHtml(message.subject)}`);
  fields.push(
    `Date: ${escapeHtml(format(new Date(message.date * 1000), "MMM d yyyy, 'at' h:mm a"))}`,
  );
  if (message.to.length) fields.push(`To: ${list(message.to)}`);
  if (message.cc.length) fields.push(`Cc: ${list(message.cc)}`);
  return [
    '<div class="gmail_quote">',
    '---------- Forwarded Message ---------<br><br>',
    fields.join('<br>'),
    '<br><br>',
    quotableBody(message),
    '<br></div>',
  ].join('\n');
}

export function replyDraft(args: {
  message: MessageJSON;
  identity: Identity;
  own: Set<string>;
  mode: ReplyMode;
}): DraftState {
  const { message, identity, own, mode } = args;
  const { to, cc } = participantsForReply(message, own, mode);
  const hasBody = message.body != null;
  return {
    ...base(identity),
    to,
    cc,
    subject: subjectWithPrefix(message.subject, 'Re:'),
    threadId: message.threadId,
    replyToHeaderMessageId: message.hMsgId,
    quoted: hasBody ? quoteForReply(message) : null,
    awaitingQuote: !hasBody,
  };
}

export function forwardDraft(args: { message: MessageJSON; identity: Identity }): DraftState {
  const { message, identity } = args;
  const hasBody = message.body != null;
  return {
    ...base(identity),
    subject: subjectWithPrefix(message.subject, 'Fwd:'),
    threadId: message.threadId,
    forwardedHeaderMessageId: message.hMsgId,
    // TODO(engine): forwarding re-attaches the original's File records by id; the engine
    // has to copy the parts from the source message when it builds the MIME message.
    files: message.files.filter((f) => !f.contentId).map((file) => ({ file })),
    quoted: hasBody ? quoteForForward(message) : null,
    awaitingQuote: !hasBody,
  };
}

/** Fills in the quote once the source message's body has arrived. */
export function withQuoteFrom(draft: DraftState, message: MessageJSON): DraftState {
  if (!draft.awaitingQuote || message.body == null) return draft;
  const quoted = draft.forwardedHeaderMessageId ? quoteForForward(message) : quoteForReply(message);
  return { ...draft, quoted, awaitingQuote: false };
}

/** Reopens a saved draft, separating what the user wrote from the quoted original. */
export function draftFromMessage(message: MessageJSON): DraftState {
  const raw = message.body ?? '';
  const { editor, quoted } = message.plaintext
    ? { editor: textToHtml(raw), quoted: null }
    : splitQuoted(raw);
  return {
    id: message.id,
    headerMessageId: message.hMsgId,
    accountId: message.aid,
    from: message.from[0] ?? { email: '' },
    to: message.to,
    cc: message.cc,
    bcc: message.bcc,
    subject: message.subject,
    body: editor,
    quoted,
    awaitingQuote: false,
    includeQuoted: true,
    threadId: message.threadId || undefined,
    replyToHeaderMessageId: message.rthMsgId ?? undefined,
    forwardedHeaderMessageId: message.fwdMsgId ?? undefined,
    files: message.files.map((file) => ({ file })),
    metadata: message.metadata ?? [],
  };
}

/** Moves a draft to another sending identity. A different account means a new draft there. */
export function changeIdentity(draft: DraftState, identity: Identity): DraftState {
  if (identity.accountId === draft.accountId) return { ...draft, from: identity.contact };
  return {
    ...draft,
    id: generateId(),
    headerMessageId: generateHeaderMessageId(identity.contact.email.split('@')[1]),
    accountId: identity.accountId,
    from: identity.contact,
    // The thread belongs to the old account; the reply headers still thread it for recipients.
    threadId: undefined,
    files: draft.files.filter((f) => f.local),
  };
}

export function fullBody(draft: DraftState): string {
  if (!draft.quoted || !draft.includeQuoted) return draft.body;
  return `${draft.body}<br><br>${draft.quoted}`;
}

export function draftToMessage(draft: DraftState): MessageJSON {
  return {
    id: draft.id,
    aid: draft.accountId,
    v: 0,
    __cls: 'Message',
    subject: draft.subject,
    date: nowSeconds(),
    threadId: draft.threadId ?? '',
    hMsgId: draft.headerMessageId,
    rthMsgId: draft.replyToHeaderMessageId ?? null,
    fwdMsgId: draft.forwardedHeaderMessageId ?? null,
    unread: false,
    starred: false,
    draft: true,
    plaintext: false,
    body: fullBody(draft),
    from: [draft.from],
    to: draft.to,
    cc: draft.cc,
    bcc: draft.bcc,
    replyTo: [],
    files: draft.files.map((f) => ({ ...f.file, messageId: draft.id, aid: draft.accountId })),
    metadata: draft.metadata,
  };
}

export function allRecipients(draft: DraftState): ContactRef[] {
  return [...draft.to, ...draft.cc, ...draft.bcc];
}

/** Window title: the subject, else the first recipient, else "New message". */
export function draftTitle(draft: DraftState | null): string {
  if (!draft) return 'New message';
  if (draft.subject.trim()) return draft.subject.trim();
  const first = draft.to[0];
  return first ? `To ${contactDisplayName(first)}` : 'New message';
}

export function setMetadata(draft: DraftState, pluginId: string, value: unknown): DraftState {
  const rest = draft.metadata.filter((m) => m.pluginId !== pluginId);
  const prev = draft.metadata.find((m) => m.pluginId === pluginId);
  return { ...draft, metadata: [...rest, { pluginId, v: (prev?.v ?? 0) + 1, value }] };
}
