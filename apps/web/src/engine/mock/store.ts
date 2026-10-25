// In-memory stand-in for the engine's SQLite store: models, thread recomputation from
// messages (Thread::apply_message_attribute_changes in tern-store), and the read API.
import { compareMessages, compareThreads, sortsAfter, threadInCategory } from '../model';
import {
  RpcError,
  RpcErrorCode,
  type CategoryCount,
  type ContactsSearchParams,
  type ContactsSearchResult,
  type MessagesGetParams,
  type MessagesGetResult,
  type SearchParams,
  type SearchResult,
  type ThreadsGetParams,
  type ThreadsGetResult,
  type ThreadsListParams,
  type ThreadsListResult,
} from '../rpc';
import type {
  AccountJSON,
  CalendarJSON,
  CategoryJSON,
  CategoryRole,
  ContactJSON,
  ContactRef,
  EventJSON,
  FolderJSON,
  LabelJSON,
  MessageJSON,
  TaskJSON,
  ThreadCategoryRef,
  ThreadJSON,
} from '../types';
import { PlacementFlags } from '../types';
import { ChangeSet } from './changes';
import { buildFixtures, FixtureBuilder, incoming, xgmName, type MockBody } from './fixtures';
import type { ThreadScript } from './fixtures/scripts';
import { grammarMatcher, plainMatcher, type Searchable } from './search';

const DEFAULT_LIMIT = 50;
const MAX_LIMIT = 500;

const clamp = (limit?: number | null) => Math.min(Math.max(limit ?? DEFAULT_LIMIT, 1), MAX_LIMIT);

function invalid(why: string): never {
  throw new RpcError({ code: RpcErrorCode.InvalidParams, message: `invalid params: ${why}` });
}

function notFound(what: string, id: string): never {
  throw new RpcError({ code: RpcErrorCode.NotFound, message: `no ${what} ${id}` });
}

/** The engine's cursor wire form, `lmt:id` (tern-store `Cursor::encode`). */
function encodeCursor(t: ThreadJSON): string {
  return `${t.lmt}:${t.id}`;
}

function decodeCursor(s: string): Pick<ThreadJSON, 'lmt' | 'id'> {
  const i = s.indexOf(':');
  const lmt = Number(s.slice(0, i));
  if (i < 0 || !Number.isInteger(lmt)) invalid('before is not a cursor');
  return { lmt, id: s.slice(i + 1) };
}

export class MockStore {
  readonly accounts = new Map<string, AccountJSON>();
  readonly categories = new Map<string, CategoryJSON>();
  readonly messages = new Map<string, MessageJSON>();
  readonly threads = new Map<string, ThreadJSON>();
  readonly contacts = new Map<string, ContactJSON>();
  readonly calendars = new Map<string, CalendarJSON>();
  readonly events = new Map<string, EventJSON>();
  readonly tasks = new Map<string, TaskJSON>();
  readonly bodies: Map<string, MockBody>;
  readonly loadedBodies: Set<string>;
  private readonly incomingQueue: ThreadScript[] = [...incoming];
  private readonly incomingBuilder: FixtureBuilder;

  constructor(now: number) {
    const f = buildFixtures(now);
    for (const a of f.accounts) this.accounts.set(a.id, a);
    for (const c of f.categories) this.categories.set(c.id, c);
    for (const m of f.messages) this.messages.set(m.id, m);
    for (const c of f.contacts) this.contacts.set(c.id, c);
    for (const c of f.calendars) this.calendars.set(c.id, c);
    for (const e of f.events) this.events.set(e.id, e);
    this.bodies = f.bodies;
    this.loadedBodies = f.loadedBodies;
    this.incomingBuilder = new FixtureBuilder(f.categories, 900);
    for (const threadId of new Set(f.messages.map((m) => m.threadId))) {
      this.recomputeThread(threadId);
    }
  }

  // ------------------------------------------------------------------ lookups

  account(id: string): AccountJSON | undefined {
    return this.accounts.get(id);
  }

  isGmail(accountId: string): boolean {
    return this.accounts.get(accountId)?.provider === 'gmail';
  }

  categoryByRole(accountId: string, role: CategoryRole): CategoryJSON | undefined {
    for (const c of this.categories.values()) {
      if (c.aid === accountId && c.role === role) return c;
    }
    return undefined;
  }

  labelByXgm(accountId: string, name: string): LabelJSON | undefined {
    for (const c of this.categories.values()) {
      if (c.aid === accountId && c.__cls === 'Label' && xgmName(c) === name) return c as LabelJSON;
    }
    return undefined;
  }

  threadMessages(threadId: string): MessageJSON[] {
    return [...this.messages.values()].filter((m) => m.threadId === threadId).sort(compareMessages);
  }

  ownEmails(accountId: string): Set<string> {
    const a = this.accounts.get(accountId);
    return new Set(a ? [a.emailAddress, ...a.aliases].map((e) => e.toLowerCase()) : []);
  }

  /** The stored message JSON, which never carries `body` (that lives in MessageBody). */
  wire(m: MessageJSON): MessageJSON {
    const out = structuredClone(m);
    delete out.body;
    const b = this.bodies.get(m.id);
    if (b) out.plaintext = b.plaintext;
    return out;
  }

  /** The body once it has been downloaded, else null. */
  body(id: string): string | null {
    const b = this.bodies.get(id);
    return b && this.loadedBodies.has(id) ? b.body : null;
  }

  /** The message with its body, as a delta right after a download carries it. */
  withBody(m: MessageJSON): MessageJSON {
    const out = this.wire(m);
    const body = this.body(m.id);
    if (body !== null) out.body = body;
    return out;
  }

  // ------------------------------------------------------------------ writes

  setMessageFlag(m: MessageJSON, flag: 'unread' | 'starred', value: boolean) {
    m[flag] = value;
    const bit = flag === 'unread' ? PlacementFlags.Unread : PlacementFlags.Starred;
    for (const id of Object.keys(m.folders ?? {})) {
      m.folders![id] = value ? m.folders![id] | bit : m.folders![id] & ~bit;
    }
    m.v += 1;
  }

  messageBits(m: MessageJSON): number {
    return (
      (m.unread ? PlacementFlags.Unread : 0) |
      (m.starred ? PlacementFlags.Starred : 0) |
      (m.draft ? PlacementFlags.Draft : 0)
    );
  }

  removeMessage(m: MessageJSON, changes: ChangeSet) {
    this.messages.delete(m.id);
    this.bodies.delete(m.id);
    this.loadedBodies.delete(m.id);
    changes.unpersist('Message', m);
  }

  /** Rebuilds a thread from its messages and records the result in `changes`. */
  recomputeThread(threadId: string, changes?: ChangeSet): ThreadJSON | null {
    const msgs = this.threadMessages(threadId);
    const existing = this.threads.get(threadId);
    if (msgs.length === 0) {
      if (existing) {
        this.threads.delete(threadId);
        changes?.unpersist('Thread', existing);
      }
      return null;
    }
    const aid = msgs[0].aid;
    const own = this.ownEmails(aid);
    const fromSelf = (m: MessageJSON) => m.from.some((c) => own.has(c.email.toLowerCase()));

    const folderRefs = new Map<string, ThreadCategoryRef>();
    const labelRefs = new Map<string, ThreadCategoryRef>();
    const bump = (map: Map<string, ThreadCategoryRef>, cat: CategoryJSON, unread: boolean) => {
      const ref: ThreadCategoryRef & { localStatus?: unknown } = map.get(cat.id) ?? {
        ...structuredClone(cat),
        _refs: 0,
        _u: 0,
      };
      delete ref.localStatus;
      ref._refs += 1;
      if (unread) ref._u += 1;
      map.set(cat.id, ref);
    };

    const participants = new Map<string, ContactRef>();
    let lmrt = 0;
    let lmst = 0;
    let attachmentCount = 0;
    for (const m of msgs) {
      for (const [fid, bits] of Object.entries(m.folders ?? {})) {
        const cat = this.categories.get(fid);
        if (cat) bump(folderRefs, cat, (bits & PlacementFlags.Unread) !== 0);
      }
      for (const name of m.labels ?? []) {
        const label = this.labelByXgm(aid, name);
        if (label) bump(labelRefs, label, m.unread);
      }
      for (const c of [...m.from, ...m.to, ...m.cc]) {
        const key = c.email.toLowerCase();
        if (!participants.has(key)) participants.set(key, { ...c });
      }
      attachmentCount += m.files.filter((f) => !f.contentId).length;
      if (!m.draft) {
        if (fromSelf(m)) lmst = Math.max(lmst, m.date);
        else lmrt = Math.max(lmrt, m.date);
      }
    }
    const first = msgs[0];
    const last = msgs[msgs.length - 1];
    const folders = [...folderRefs.values()];
    const thread: ThreadJSON = {
      id: threadId,
      aid,
      v: (existing?.v ?? 0) + 1,
      __cls: 'Thread',
      subject: first.subject,
      snippet: last.snippet ?? '',
      unread: msgs.filter((m) => m.unread).length,
      starred: msgs.filter((m) => m.starred).length,
      lmt: last.date,
      fmt: first.date,
      lmst,
      lmrt: lmrt || last.date,
      gThrId: existing?.gThrId ?? (this.isGmail(aid) ? first.gMsgId : undefined),
      inAllMail: !folders.every((f) => f.role === 'spam' || f.role === 'trash'),
      attachmentCount,
      folders,
      labels: [...labelRefs.values()],
      participants: [...participants.values()],
      metadata: existing?.metadata,
    };
    if (thread.gThrId === undefined) delete thread.gThrId;
    if (thread.metadata === undefined) delete thread.metadata;
    this.threads.set(threadId, thread);
    changes?.persist('Thread', thread);
    return thread;
  }

  /** Adds the next scripted incoming thread, if any is left. */
  deliverIncoming(now: number, accountId?: string): ChangeSet | null {
    const idx = this.incomingQueue.findIndex(
      (s) => !accountId || this.accountIdFor(s) === accountId,
    );
    if (idx === -1) return null;
    const [script] = this.incomingQueue.splice(idx, 1);
    const before = this.incomingBuilder.messages.length;
    const threadId = this.incomingBuilder.add(script, now);
    const changes = new ChangeSet(this.accountIdFor(script));
    for (const m of this.incomingBuilder.messages.slice(before)) {
      this.messages.set(m.id, m);
      this.bodies.set(m.id, this.incomingBuilder.bodies.get(m.id)!);
      this.loadedBodies.add(m.id);
      changes.persist('Message', m);
    }
    this.recomputeThread(threadId, changes);
    return changes;
  }

  private accountIdFor(script: ThreadScript): string {
    return [...this.accounts.values()].find((a) =>
      script.account === 'gmail' ? a.provider === 'gmail' : a.provider !== 'gmail',
    )!.id;
  }

  // ------------------------------------------------------------------ read API

  listCategories(accountId: string, cls: 'Folder'): FolderJSON[];
  listCategories(accountId: string, cls: 'Label'): LabelJSON[];
  listCategories(accountId: string, cls: 'Folder' | 'Label'): CategoryJSON[] {
    return [...this.categories.values()]
      .filter((c) => c.aid === accountId && c.__cls === cls)
      .sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0))
      .map((c) => structuredClone(c));
  }

  listThreads(p: ThreadsListParams): ThreadsListResult {
    const filters = [p.folderId, p.labelId, p.role].filter((f) => f !== undefined);
    if (filters.length > 1) invalid('give at most one of folderId, labelId and role');
    if (p.role === '') invalid('role is empty');
    let category: CategoryJSON | undefined;
    if (p.folderId || p.labelId) {
      const id = (p.folderId ?? p.labelId)!;
      category = this.categories.get(id);
      const cls = p.folderId ? 'Folder' : 'Label';
      if (!category || category.aid !== p.accountId || category.__cls !== cls) {
        notFound(cls.toLowerCase(), id);
      }
    } else if (p.role) {
      category =
        this.categoryOfClass(p.accountId, p.role, 'Folder') ??
        this.categoryOfClass(p.accountId, p.role, 'Label');
      if (!category) notFound('folder or label with role', p.role);
    }
    let list = [...this.threads.values()].filter((t) => t.aid === p.accountId);
    if (category) {
      const cat = category;
      // Outside spam and trash a category lists only threads in all mail (tern-store threads()).
      const anywhere = cat.role === 'spam' || cat.role === 'trash';
      list = list.filter((t) => threadInCategory(t, cat.id) && (anywhere || t.inAllMail));
    }
    const sorted = list.sort(compareThreads);
    const after = p.before ? decodeCursor(p.before) : null;
    const start = after ? sorted.filter((t) => sortsAfter(t, after)) : sorted;
    const limit = clamp(p.limit);
    const threads = start.slice(0, limit);
    const next = start.length > limit ? encodeCursor(threads[threads.length - 1]) : null;
    return { threads: threads.map((t) => structuredClone(t)), next };
  }

  private categoryOfClass(accountId: string, role: string, cls: 'Folder' | 'Label') {
    return [...this.categories.values()]
      .filter((c) => c.aid === accountId && c.__cls === cls && c.role === role)
      .sort((a, b) => (a.id < b.id ? -1 : 1))[0];
  }

  getThread(p: ThreadsGetParams): ThreadsGetResult {
    const thread = this.threads.get(p.id);
    if (!thread || thread.aid !== p.accountId) notFound('thread', p.id);
    return {
      thread: structuredClone(thread),
      messages: this.threadMessages(p.id).map((m) => this.wire(m)),
    };
  }

  getMessage(p: MessagesGetParams): MessagesGetResult {
    const m = this.messages.get(p.id);
    if (!m || m.aid !== p.accountId) notFound('message', p.id);
    const message = this.wire(m);
    return { message, body: p.body ? this.body(m.id) : null, files: message.files };
  }

  /** A thread's `ThreadSearch` row and flags, as `search` matches them. */
  searchable(t: ThreadJSON): Searchable {
    const msgs = this.threadMessages(t.id);
    const people = (cs: ContactRef[]) => cs.flatMap((c) => [c.email, c.name ?? '']).join(' ');
    return {
      columns: {
        subject: t.subject,
        from: msgs.map((m) => people(m.from)).join(' '),
        to: msgs.map((m) => people([...m.to, ...m.cc, ...m.bcc])).join(' '),
        categories: [...t.folders, ...t.labels]
          .filter((c) => c._refs > 0)
          .map((c) => c.role || c.path)
          .join(' '),
        body: msgs.map((m) => `${m.snippet ?? ''} ${this.bodies.get(m.id)?.body ?? ''}`).join(' '),
      },
      unread: t.unread,
      starred: t.starred,
      attachmentCount: t.attachmentCount,
      lmrt: t.lmrt,
    };
  }

  /** `search` over `accounts`, which the caller has checked. */
  search(p: SearchParams, accounts: string[]): SearchResult {
    const match = p.grammar
      ? grammarMatcher(p.query, p.tz ?? undefined, p.now ?? undefined)
      : plainMatcher(p.query);
    const inScope = new Set(accounts);
    return {
      threads: [...this.threads.values()]
        .filter((t) => inScope.has(t.aid) && match(this.searchable(t)))
        .sort(compareThreads)
        .slice(0, clamp(p.limit))
        .map((t) => structuredClone(t)),
    };
  }

  searchContacts(p: ContactsSearchParams): ContactsSearchResult {
    const terms = p.query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (terms.length === 0) return { contacts: [] };
    const words = (c: ContactJSON) => `${c.email} ${c.name}`.toLowerCase().split(/[\s@.]+/);
    const contacts = [...this.contacts.values()]
      .filter((c) => c.aid === p.accountId && !c.h)
      .filter((c) => terms.every((t) => words(c).some((w) => w.startsWith(t))))
      .sort((a, b) => b.refs - a.refs || (a.id < b.id ? -1 : 1))
      .slice(0, clamp(p.limit))
      .map((c) => structuredClone(c));
    return { contacts };
  }

  /** The ThreadCounts rows of the account's folders or labels. */
  counts(accountId: string, cls: 'Folder' | 'Label'): Record<string, CategoryCount> {
    const out: Record<string, CategoryCount> = {};
    for (const c of this.categories.values()) {
      if (c.aid === accountId && c.__cls === cls) out[c.id] = { unread: 0, total: 0 };
    }
    for (const t of this.threads.values()) {
      if (t.aid !== accountId) continue;
      for (const ref of [...t.folders, ...t.labels]) {
        const slot = out[ref.id];
        if (!slot || ref._refs === 0) continue;
        slot.total += 1;
        if (ref._u > 0) slot.unread += 1;
      }
    }
    return out;
  }
}
