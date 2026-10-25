// The composer's working copy of a draft and how it maps to the engine's Message JSON. The
// engine builds new drafts, replies and forwards (`draft.create`); this splits one into what
// the editor shows and the quoted original, and back.
import {
  contactDisplayName,
  generateHeaderMessageId,
  generateId,
  nowSeconds,
  type ContactRef,
  type FileJSON,
  type MessageJSON,
  type PluginMetadataJSON,
} from '@/engine';
import { splitQuoted, textToHtml } from './html';
import type { Identity } from './identities';

export interface DraftAttachment {
  file: FileJSON;
  /** Bytes picked in this browser. */
  local?: File;
  /**
   * The bytes are on their way to the engine (`files.put` through the gateway), or didn't
   * get there. Until then `file` is a placeholder that is neither saved nor sent.
   */
  upload?: 'pending' | 'failed';
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
  includeQuoted: boolean;
  threadId?: string;
  replyToHeaderMessageId?: string;
  forwardedHeaderMessageId?: string;
  files: DraftAttachment[];
  metadata: PluginMetadataJSON[];
}

/** A draft for the composer, separating what the user wrote from the quoted original. */
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
  const id = generateId();
  return {
    ...draft,
    id,
    headerMessageId: generateHeaderMessageId(identity.contact.email.split('@')[1]),
    accountId: identity.accountId,
    from: identity.contact,
    // The thread belongs to the old account; the reply headers still thread it for recipients.
    threadId: undefined,
    // The engine keeps a File under the account it was put to, so picked files go up again
    // under the new one (the session's `upload`). Fresh placeholder ids let an upload still running
    // for the old account land nowhere.
    files: draft.files
      .filter((f) => f.local)
      .map((f) => ({
        local: f.local,
        upload: 'pending' as const,
        file: { ...f.file, id: generateId(), aid: identity.accountId, messageId: id },
      })),
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
    files: draft.files
      .filter((f) => !f.upload)
      .map((f) => ({ ...f.file, messageId: draft.id, aid: draft.accountId })),
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
