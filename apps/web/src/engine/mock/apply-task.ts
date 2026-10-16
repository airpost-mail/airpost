// Local phase of each task against the mock store. The remote phase is simulated by the
// engine wrapper (status `remote` -> `complete` after a delay).
// TODO(engine): every branch here stands in for the engine's ms-tasks implementation;
// delete this file when the WebSocket client is the default.
import { generateId, nowSeconds } from '../model';
import type {
  CategoryJSON,
  ChangeFolderTaskJSON,
  ChangeLabelsTaskJSON,
  MessageJSON,
  SendDraftTaskJSON,
  SyncbackCategoryTaskJSON,
  SyncbackDraftTaskJSON,
  SyncbackMetadataTaskJSON,
  TaskJSON,
} from '../types';
import { PlacementFlags } from '../types';
import { ChangeSet } from './changes';
import { snippetOf, xgmName } from './fixtures';
import type { MockStore } from './store';

export interface TaskOutcome {
  changes: ChangeSet;
  /** How long the simulated remote phase takes. */
  remoteMs: number;
  /**
   * Effects that only show once the remote phase is done: tasks the engine has no local
   * phase for (ms-tasks `perform_local_by_class` returns Ok for them) change the store
   * through the next folder sync. Called with the stored task when it completes.
   */
  remote?: (task: TaskJSON) => ChangeSet;
  error?: TaskJSON['error'];
}

export function applyTask(store: MockStore, task: TaskJSON): TaskOutcome {
  const changes = new ChangeSet(task.aid);
  const touched = new Set<string>();
  const persistMsg = (m: MessageJSON) => {
    changes.persist('Message', m);
    touched.add(m.threadId);
  };
  const threadMsgs = (threadIds: string[], messageIds: string[] = []) => {
    const ids = new Set(messageIds);
    return threadIds
      .flatMap((id) => store.threadMessages(id))
      .concat([...ids].map((id) => store.messages.get(id)).filter((m): m is MessageJSON => !!m))
      .filter((m, i, all) => all.findIndex((x) => x.id === m.id) === i && m.aid === task.aid);
  };

  let remoteMs = 250;
  let error: TaskJSON['error'];

  switch (task.__cls) {
    case 'ChangeUnreadTask':
      for (const m of threadMsgs(task.threadIds, task.messageIds)) {
        if (m.unread === task.unread) continue;
        store.setMessageFlag(m, 'unread', task.unread);
        persistMsg(m);
      }
      break;

    case 'ChangeStarredTask': {
      // Starring a thread stars its newest message; unstarring clears every message.
      for (const threadId of task.threadIds) {
        const msgs = store.threadMessages(threadId);
        const targets = task.starred ? msgs.slice(-1) : msgs.filter((m) => m.starred);
        for (const m of targets) {
          store.setMessageFlag(m, 'starred', task.starred);
          persistMsg(m);
        }
      }
      break;
    }

    case 'ChangeFolderTask':
      for (const m of threadMsgs(task.threadIds, task.messageIds)) {
        if (moveMessage(store, m, task)) persistMsg(m);
      }
      break;

    case 'ChangeLabelsTask':
      if (!store.isGmail(task.aid)) break;
      for (const m of threadMsgs(task.threadIds, task.messageIds)) {
        if (relabelMessage(m, task)) persistMsg(m);
      }
      break;

    case 'DestroyDraftTask':
      for (const id of task.messageIds) {
        const m = store.messages.get(id);
        if (!m || !m.draft) continue;
        touched.add(m.threadId);
        store.removeMessage(m, changes);
      }
      break;

    case 'SyncbackDraftTask':
      persistMsg(saveDraft(store, task));
      remoteMs = 150;
      break;

    case 'SendDraftTask': {
      const d = task.draft;
      if (d.to.length + d.cc.length + d.bcc.length === 0) {
        error = {
          key: 'no-recipients',
          debuginfo: 'The draft has no recipients.',
          retryable: false,
        };
        break;
      }
      const sent = sendDraft(store, task, changes);
      persistMsg(sent);
      touched.add(sent.threadId);
      remoteMs = 900;
      break;
    }

    case 'SyncbackMetadataTask':
      applyMetadata(store, task, changes);
      break;

    case 'ExpungeAllInFolderTask':
    case 'SyncbackCategoryTask':
    case 'DestroyCategoryTask':
      remoteMs = task.__cls === 'ExpungeAllInFolderTask' ? 600 : 250;
      return { changes, remoteMs, remote: (stored) => applyRemote(store, stored) };

    case 'ChangeRoleMappingTask': {
      const cat = [...store.categories.values()].find(
        (c) => c.aid === task.aid && c.path === task.path,
      );
      if (cat) {
        cat.role = task.role as CategoryJSON['role'];
        cat.v += 1;
        changes.persist(cat.__cls, cat as never);
      }
      break;
    }

    default:
      // TODO(engine): GetMessageRFC2822Task, EventRSVPTask and the contact tasks have no
      // mock behaviour; they complete without changing anything.
      break;
  }

  for (const threadId of touched) store.recomputeThread(threadId, changes);
  return { changes, remoteMs, error };
}

/** The remote phase of the tasks that have no local one, as the next sync would report it. */
function applyRemote(store: MockStore, task: TaskJSON): ChangeSet {
  const changes = new ChangeSet(task.aid);
  const touched = new Set<string>();
  const persistMsg = (m: MessageJSON) => {
    changes.persist('Message', m);
    touched.add(m.threadId);
  };
  switch (task.__cls) {
    case 'ExpungeAllInFolderTask':
      for (const m of [...store.messages.values()]) {
        if (m.aid !== task.aid || !(task.folder.id in (m.folders ?? {}))) continue;
        delete m.folders![task.folder.id];
        touched.add(m.threadId);
        if (Object.keys(m.folders!).length === 0) store.removeMessage(m, changes);
        else persistMsg(m);
      }
      break;

    case 'SyncbackCategoryTask':
      for (const id of saveCategory(store, task, changes)) touched.add(id);
      break;

    case 'DestroyCategoryTask': {
      const cat = [...store.categories.values()].find(
        (c) => c.aid === task.aid && c.path === task.path,
      );
      if (!cat) break;
      store.categories.delete(cat.id);
      changes.unpersist(cat.__cls, cat as never);
      for (const m of [...store.messages.values()]) {
        if (m.aid !== task.aid) continue;
        if (cat.__cls === 'Label' && m.labels?.includes(xgmName(cat))) {
          m.labels = m.labels.filter((l) => l !== xgmName(cat));
          m.v += 1;
          persistMsg(m);
        } else if (m.folders && cat.id in m.folders) {
          touched.add(m.threadId);
          store.removeMessage(m, changes);
        }
      }
      break;
    }
  }
  for (const threadId of touched) store.recomputeThread(threadId, changes);
  return changes;
}

function moveMessage(store: MockStore, m: MessageJSON, task: ChangeFolderTaskJSON): boolean {
  const target = task.folder;
  const bits = store.messageBits(m);
  const current = Object.keys(m.folders ?? {});
  if (store.isGmail(m.aid)) {
    if (current.length === 1 && current[0] === target.id) return false;
    m.folders = { [target.id]: bits };
    // Gmail keeps labels on trashed mail but hides them; the engine reports none.
    if (target.role === 'trash' || target.role === 'spam') m.labels = [];
    m.v += 1;
    return true;
  }
  const lockedRoles = new Set(['sent', 'drafts']);
  const sources = task.sourceFolderIds?.length
    ? current.filter((id) => task.sourceFolderIds!.includes(id))
    : current.filter((id) => !lockedRoles.has(store.categories.get(id)?.role ?? ''));
  if (sources.length === 0 || (sources.length === 1 && sources[0] === target.id)) return false;
  const next: Record<string, number> = {};
  for (const id of current) if (!sources.includes(id)) next[id] = m.folders![id];
  next[target.id] = bits;
  m.folders = next;
  m.v += 1;
  return true;
}

function relabelMessage(m: MessageJSON, task: ChangeLabelsTaskJSON): boolean {
  const before = new Set(m.labels ?? []);
  const next = new Set(before);
  for (const l of task.labelsToRemove) next.delete(xgmName(l));
  for (const l of task.labelsToAdd) next.add(xgmName(l));
  if (next.size === before.size && [...next].every((l) => before.has(l))) return false;
  m.labels = [...next];
  m.v += 1;
  return true;
}

function resolveThreadId(store: MockStore, draft: MessageJSON, existing?: MessageJSON): string {
  if (existing) return existing.threadId;
  if (draft.threadId && store.threads.has(draft.threadId)) return draft.threadId;
  if (draft.rthMsgId) {
    const parent = [...store.messages.values()].find((m) => m.hMsgId === draft.rthMsgId);
    if (parent) return parent.threadId;
  }
  return `t-n-${generateId()}`;
}

function findDraft(store: MockStore, task: SyncbackDraftTaskJSON | SendDraftTaskJSON) {
  return [...store.messages.values()].find(
    (m) =>
      m.draft &&
      m.aid === task.aid &&
      (m.hMsgId === task.headerMessageId || m.id === task.draft.id),
  );
}

function placementFor(store: MockStore, aid: string, role: 'drafts' | 'sent', bits: number) {
  if (store.isGmail(aid)) {
    const all = store.categoryByRole(aid, 'all')!;
    return { folders: { [all.id]: bits }, labels: [role === 'drafts' ? '\\Draft' : '\\Sent'] };
  }
  return { folders: { [store.categoryByRole(aid, role)!.id]: bits }, labels: [] };
}

function saveDraft(store: MockStore, task: SyncbackDraftTaskJSON): MessageJSON {
  const existing = findDraft(store, task);
  const d = task.draft;
  const body = d.body ?? '';
  const id = existing?.id ?? d.id;
  const msg: MessageJSON = {
    ...structuredClone(d),
    id,
    aid: task.aid,
    v: (existing?.v ?? 0) + 1,
    __cls: 'Message',
    threadId: resolveThreadId(store, d, existing),
    hMsgId: task.headerMessageId,
    draft: true,
    unread: false,
    starred: false,
    date: nowSeconds(),
    snippet: snippetOf(body, !!d.plaintext),
    files: d.files ?? [],
    ...placementFor(store, task.aid, 'drafts', PlacementFlags.Draft),
  };
  delete msg.body;
  store.messages.set(id, msg);
  store.bodies.set(id, { body, plaintext: !!d.plaintext });
  store.loadedBodies.add(id);
  return store.withBody(msg);
}

function sendDraft(store: MockStore, task: SendDraftTaskJSON, changes: ChangeSet): MessageJSON {
  const existing = findDraft(store, task);
  const d = task.draft;
  const threadId = resolveThreadId(store, d, existing);
  if (existing) store.removeMessage(existing, changes);
  const body = d.body ?? '';
  const id = `m-s-${generateId()}`;
  const msg: MessageJSON = {
    ...structuredClone(d),
    id,
    aid: task.aid,
    v: 1,
    __cls: 'Message',
    threadId,
    hMsgId: task.headerMessageId,
    draft: false,
    unread: false,
    starred: false,
    date: nowSeconds(),
    snippet: snippetOf(body, !!d.plaintext),
    files: d.files ?? [],
    ...placementFor(store, task.aid, 'sent', 0),
  };
  delete msg.body;
  store.messages.set(id, msg);
  store.bodies.set(id, { body, plaintext: !!d.plaintext });
  store.loadedBodies.add(id);
  return store.withBody(msg);
}

function applyMetadata(store: MockStore, task: SyncbackMetadataTaskJSON, changes: ChangeSet) {
  const model =
    task.modelClassName === 'Thread'
      ? store.threads.get(task.modelId)
      : task.modelClassName === 'Message'
        ? store.messages.get(task.modelId)
        : undefined;
  if (!model) return;
  const list = (model.metadata ??= []);
  const entry = list.find((x) => x.pluginId === task.pluginId);
  if (entry) {
    entry.value = task.value;
    entry.v += 1;
  } else {
    list.push({ pluginId: task.pluginId, v: 1, value: task.value });
  }
  model.v += 1;
  if (model.__cls === 'Thread') changes.persist('Thread', model);
  else changes.persist('Message', model);
}

/** Returns the threads whose embedded category refs need rebuilding. */
function saveCategory(store: MockStore, task: SyncbackCategoryTaskJSON, changes: ChangeSet) {
  const touched: string[] = [];
  if (task.existingPath) {
    const cat = [...store.categories.values()].find(
      (c) => c.aid === task.aid && c.path === task.existingPath,
    );
    if (!cat) return touched;
    const oldXgm = xgmName(cat);
    cat.path = task.path;
    cat.v += 1;
    changes.persist(cat.__cls, cat as never);
    for (const m of store.messages.values()) {
      if (m.aid !== task.aid) continue;
      if (cat.__cls === 'Label' && m.labels?.includes(oldXgm)) {
        m.labels = m.labels.map((l) => (l === oldXgm ? xgmName(cat) : l));
        touched.push(m.threadId);
      } else if (m.folders && cat.id in m.folders) {
        touched.push(m.threadId);
      }
    }
    return touched;
  }
  const cls = store.isGmail(task.aid) ? 'Label' : 'Folder';
  const cat: CategoryJSON = {
    id: `${cls === 'Label' ? 'g' : 'i'}-${generateId().slice(0, 8)}`,
    aid: task.aid,
    v: 1,
    __cls: cls,
    path: task.path,
    role: '',
    localStatus: {},
  };
  store.categories.set(cat.id, cat);
  task.created = structuredClone(cat);
  changes.persist(cls, cat as never);
  return touched;
}
