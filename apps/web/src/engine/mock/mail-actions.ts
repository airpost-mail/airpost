// `threads.snooze`, `threads.unsnooze` and `messages.unsubscribe` against the mock store, as
// the engine does them (crates/tern-tasks/src/mail_actions.rs, crates/tern-net/src/unsubscribe.rs):
// the same tasks, the same choice of unsubscribe method. The mock makes no HTTP request.
import { generateHeaderMessageId, generateId, nowSeconds, threadHasRole } from '../model';
import { RpcError, RpcErrorCode, type UnsubscribeMethod } from '../rpc';
import * as tasks from './tasks';
import type {
  AccountJSON,
  FolderJSON,
  LabelJSON,
  MessageJSON,
  TaskJSON,
  ThreadJSON,
} from '../types';
import type { MockStore } from './store';

export const THREAD_SNOOZE = 'thread-snooze';
const ONE_CLICK_BODY = 'List-Unsubscribe=One-Click';
const MAILTO_MAX_RECIPIENTS = 5;

const notFound = (message: string) => new RpcError({ code: RpcErrorCode.NotFound, message });

function findThreads(store: MockStore, accountId: string, ids: string[]): ThreadJSON[] {
  if (ids.length === 0) throw notFound('no threads given');
  const out: ThreadJSON[] = [];
  for (const id of new Set(ids)) {
    const t = store.threads.get(id);
    if (!t || t.aid !== accountId) throw notFound(`no thread ${id}`);
    out.push(t);
  }
  return out;
}

function snoozeCategories(store: MockStore, accountId: string) {
  const snoozed = store.categoryByRole(accountId, 'snoozed');
  if (!snoozed) {
    throw notFound('the account has no Snoozed folder yet; it is created by the first sync');
  }
  return { snoozed, inbox: store.categoryByRole(accountId, 'inbox') };
}

const metadata = (accountId: string, t: ThreadJSON, expiration: number | null) =>
  tasks.syncbackMetadata({
    accountId,
    pluginId: THREAD_SNOOZE,
    modelId: t.id,
    modelClassName: 'thread',
    value: { expiration },
  });

/** The move to Snoozed, then one `thread-snooze` metadata task per thread. */
export function snoozeTasks(
  store: MockStore,
  accountId: string,
  ids: string[],
  until: number,
): TaskJSON[] {
  if (until <= nowSeconds()) {
    throw new RpcError({
      code: RpcErrorCode.InvalidParams,
      message: 'invalid params: until is not in the future',
    });
  }
  const threads = findThreads(store, accountId, ids);
  const { snoozed, inbox } = snoozeCategories(store, accountId);
  const target = { accountId, threadIds: threads.map((t) => t.id), source: 'Snooze Move' };
  const move =
    snoozed.__cls === 'Label'
      ? tasks.changeLabels({
          ...target,
          labelsToAdd: [snoozed as LabelJSON],
          labelsToRemove: inbox?.__cls === 'Label' ? [inbox as LabelJSON] : [],
        })
      : tasks.changeFolder({
          ...target,
          folder: snoozed as FolderJSON,
          sourceFolderIds: inbox ? [inbox.id] : [],
        });
  return [move, ...threads.map((t) => metadata(accountId, t, until))];
}

/** The move back for the threads still in Snoozed, then the metadata cleared on all. */
export function unsnoozeTasks(store: MockStore, accountId: string, ids: string[]): TaskJSON[] {
  const threads = findThreads(store, accountId, ids);
  const { snoozed, inbox } = snoozeCategories(store, accountId);
  const still = threads.filter((t) => threadHasRole(t, 'snoozed'));
  const out: TaskJSON[] = [];
  if (still.length > 0) {
    if (!inbox) throw notFound('the account has no Inbox folder');
    const target = {
      accountId,
      threadIds: still.map((t) => t.id),
      source: 'Snooze Move',
      canBeUndone: false,
    };
    out.push(
      snoozed.__cls === 'Label'
        ? tasks.changeLabels({
            ...target,
            labelsToAdd: [inbox as LabelJSON],
            labelsToRemove: [snoozed as LabelJSON],
          })
        : tasks.changeFolder({
            ...target,
            folder: inbox as FolderJSON,
            sourceFolderIds: [snoozed.id],
          }),
    );
  }
  return [...out, ...threads.map((t) => metadata(accountId, t, null))];
}

/** The `<…>` URIs of a `List-Unsubscribe` value, http(s) and mailto only, in order. */
export function headerUris(header: string | undefined): string[] {
  const out: string[] = [];
  for (const m of (header ?? '').matchAll(/<([^>]*)>/g)) {
    const uri = m[1]!.replace(/\s+/g, '');
    if (/^(https?:\/\/|mailto:)/i.test(uri)) out.push(uri);
  }
  return out;
}

/** The method the engine uses: `wanted`, or one-click, else mailto, else web. */
export function chooseUnsubscribe(
  header: string | undefined,
  post: string | undefined,
  wanted?: UnsubscribeMethod,
): { method: UnsubscribeMethod; url: string } | null {
  const uris = headerUris(header);
  const has = (prefix: string) => uris.find((u) => u.toLowerCase().startsWith(prefix));
  const pick = (method: UnsubscribeMethod, url: string | undefined) =>
    url ? { method, url } : null;
  const oneClick = () =>
    post?.trim() === ONE_CLICK_BODY ? pick('one-click', has('https://')) : null;
  const mailto = () => pick('mailto', has('mailto:'));
  const web = () => pick('web', has('https://') ?? has('http://'));
  switch (wanted) {
    case 'one-click':
      return oneClick();
    case 'mailto':
      return mailto();
    case 'web':
      return web();
    default:
      return oneClick() ?? mailto() ?? web();
  }
}

export interface Mailto {
  to: string[];
  cc: string[];
  subject: string;
  body: string;
}

const ADDRESS = /^[^\s@<>()[\],;:"\\]+@[^\s@<>()[\],;:"\\]+\.[^\s@<>()[\],;:"\\]+$/;

/** RFC 6068, the fields the engine reads. Throws an Error saying why it can't be sent. */
export function parseMailto(uri: string): Mailto {
  if (!/^mailto:/i.test(uri)) throw new Error('not a mailto URI');
  const [path, query = ''] = uri.slice(7).split('?', 2) as [string, string?];
  const m: Mailto = { to: [], cc: [], subject: '', body: '' };
  const decode = (s: string) => {
    try {
      return decodeURIComponent(s);
    } catch {
      throw new Error('the mailto URI has a broken %-escape');
    }
  };
  const add = (list: string[], raw: string) => {
    for (const a of decode(raw).split(',')) {
      const addr = a.trim();
      if (!addr) continue;
      if (!ADDRESS.test(addr)) throw new Error(`"${addr}" is not an email address`);
      list.push(addr);
    }
  };
  add(m.to, path);
  for (const pair of query.split('&').filter(Boolean)) {
    const [key, value = ''] = pair.split('=', 2) as [string, string?];
    switch (key.toLowerCase()) {
      case 'to':
        add(m.to, value);
        break;
      case 'cc':
        add(m.cc, value);
        break;
      case 'subject':
        m.subject = decode(value);
        break;
      case 'body':
        m.body = decode(value);
        break;
    }
  }
  if (m.to.length === 0) throw new Error('the mailto URI names no recipient');
  if (m.to.length + m.cc.length > MAILTO_MAX_RECIPIENTS) {
    throw new Error(`the mailto URI names more than ${MAILTO_MAX_RECIPIENTS} recipients`);
  }
  // eslint-disable-next-line no-control-regex
  m.subject = m.subject.replace(/[\u0000-\u001f\u007f]/g, ' ').trim();
  return m;
}

/** The address the list wrote to: an alias among the To and Cc, else the account's own. */
function senderFor(account: AccountJSON, message: MessageJSON) {
  const received = new Set([...message.to, ...message.cc].map((c) => c.email.toLowerCase()));
  const identities = [
    { name: account.name, email: account.emailAddress },
    ...account.aliases.map((alias) => {
      const m = /^(.*)<([^>]+)>\s*$/.exec(alias);
      return m
        ? { name: m[1]!.trim().replace(/^"|"$/g, ''), email: m[2]!.trim() }
        : { name: account.name, email: alias.trim() };
    }),
  ];
  return identities.find((i) => received.has(i.email.toLowerCase())) ?? identities[0]!;
}

/** The `SendDraftTask` for an unsubscribe by mail, in plain text. */
export function unsubscribeSendTask(
  account: AccountJSON,
  message: MessageJSON,
  mailto: Mailto,
): TaskJSON {
  const from = senderFor(account, message);
  const text = (s: string) => (s.trim() ? s : 'unsubscribe');
  const contacts = (list: string[]) => list.map((email) => ({ name: '', email }));
  const draft: MessageJSON = {
    id: generateId(),
    aid: account.id,
    v: 0,
    __cls: 'Message',
    hMsgId: generateHeaderMessageId(from.email.split('@')[1] || undefined),
    threadId: '',
    subject: text(mailto.subject),
    date: nowSeconds(),
    from: [from],
    to: contacts(mailto.to),
    cc: contacts(mailto.cc),
    bcc: [],
    replyTo: [],
    files: [],
    body: text(mailto.body),
    plaintext: true,
    draft: true,
    unread: false,
    starred: false,
  };
  return tasks.sendDraft(draft);
}
