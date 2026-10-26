// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

// The `threads.*` intents against the mock store, planned as the engine's tern-tasks does
// (crates/tern-tasks/src/intents.rs): every account's tasks before any is queued, Gmail's
// labels and All Mail, the archive fallbacks. The mock keeps no undo log: its answers carry
// no token, and `undo.apply` has nothing to undo.
import { RpcError, RpcErrorCode, type RpcParams } from '../rpc';
import type { CategoryJSON, FolderJSON, LabelJSON, TaskJSON, ThreadJSON } from '../types';
import type { MockStore } from './store';
import * as tasks from './tasks';

export type MockIntent =
  | 'threads.archive'
  | 'threads.trash'
  | 'threads.spam'
  | 'threads.notSpam'
  | 'threads.move'
  | 'threads.label'
  | 'threads.markRead'
  | 'threads.star';

const notFound = (message: string) => new RpcError({ code: RpcErrorCode.NotFound, message });

const isFolder = (c: CategoryJSON | undefined): c is FolderJSON => c?.__cls === 'Folder';
const isLabel = (c: CategoryJSON | undefined): c is LabelJSON => c?.__cls === 'Label';

/** A folder or label by id, else role, else path, without regard to case. */
function resolve(list: CategoryJSON[], name: string): CategoryJSON | undefined {
  const lower = name.toLowerCase();
  return (
    list.find((c) => c.id === name) ??
    list.find((c) => c.role === lower) ??
    list.find((c) => c.path.toLowerCase() === lower)
  );
}

/** Each account's tasks for `method`, all planned before any is queued. */
export function planIntent<M extends MockIntent>(
  store: MockStore,
  method: M,
  params: RpcParams<M>,
): Array<[string, TaskJSON[]]> {
  const p = params as RpcParams<'threads.move'> &
    RpcParams<'threads.label'> &
    RpcParams<'threads.markRead'> &
    RpcParams<'threads.star'>;
  const threadIds = p.threadIds ?? [];
  if (threadIds.length === 0) throw notFound('no threads given');
  const byAccount = new Map<string, ThreadJSON[]>();
  for (const id of threadIds) {
    const t = store.threads.get(id);
    if (!t) throw notFound(`no thread ${id}`);
    byAccount.set(t.aid, [...(byAccount.get(t.aid) ?? []), t]);
  }
  const sources = (aid: string) =>
    (p.perspective?.categoryIds ?? []).filter((id) => {
      const c = store.categories.get(id);
      return c?.aid === aid && isFolder(c);
    });

  return [...byAccount].map(([aid, ts]) => {
    const target = { accountId: aid, threadIds: ts.map((t) => t.id) };
    const role = (r: CategoryJSON['role']) => store.categoryByRole(aid, r);
    const folder = (r: CategoryJSON['role'], scoped = false) => {
      const f = role(r);
      if (!isFolder(f)) throw notFound(`the account has no ${r} folder`);
      return tasks.changeFolder({
        ...target,
        folder: f,
        sourceFolderIds: scoped ? sources(aid) : [],
      });
    };
    const labels = (names: string[] = []) =>
      names.map((n) => {
        const l = resolve(store.listCategories(aid, 'Label'), n);
        if (!isLabel(l)) throw notFound(`no label ${n}`);
        return l;
      });
    switch (method) {
      case 'threads.archive': {
        const inbox = role('inbox');
        if (isLabel(inbox))
          return [aid, [tasks.changeLabels({ ...target, labelsToRemove: [inbox] })]];
        const archive = role('archive') ?? role('all');
        if (!isFolder(archive)) throw notFound('the account has no Archive folder');
        return [
          aid,
          [tasks.changeFolder({ ...target, folder: archive, sourceFolderIds: sources(aid) })],
        ];
      }
      case 'threads.trash':
        return [aid, [folder('trash')]];
      case 'threads.spam':
        return [aid, [folder('spam')]];
      case 'threads.notSpam': {
        const spam = role('spam');
        const to = isLabel(role('inbox')) ? role('all') : role('inbox');
        if (!isFolder(to)) throw notFound('the account has no folder to move it back to');
        const from = spam ? [spam.id] : [];
        return [aid, [tasks.changeFolder({ ...target, folder: to, sourceFolderIds: from })]];
      }
      case 'threads.move': {
        const f = resolve(store.listCategories(aid, 'Folder'), p.to);
        if (isFolder(f)) {
          return [
            aid,
            [tasks.changeFolder({ ...target, folder: f, sourceFolderIds: sources(aid) })],
          ];
        }
        const l = store.isGmail(aid)
          ? resolve(store.listCategories(aid, 'Label'), p.to)
          : undefined;
        if (!isLabel(l)) throw notFound(`no folder ${p.to}`);
        const others = new Map<string, LabelJSON>();
        for (const t of ts) {
          for (const r of t.labels) {
            const c = store.categories.get(r.id);
            if (isLabel(c) && c.id !== l.id) others.set(c.id, c);
          }
        }
        return [
          aid,
          [
            tasks.changeLabels({
              ...target,
              labelsToAdd: [l],
              labelsToRemove: [...others.values()],
            }),
          ],
        ];
      }
      case 'threads.label':
        if (!store.isGmail(aid)) {
          throw new RpcError({
            code: RpcErrorCode.InvalidParams,
            message: 'the account has folders, not labels (only Gmail has labels)',
          });
        }
        return [
          aid,
          [
            tasks.changeLabels({
              ...target,
              labelsToAdd: labels(p.add),
              labelsToRemove: labels(p.remove),
            }),
          ],
        ];
      case 'threads.markRead':
        return [aid, [tasks.changeUnread({ ...target, unread: !!p.unread })]];
      default:
        return [aid, [tasks.changeStarred({ ...target, starred: p.starred !== false })]];
    }
  });
}
