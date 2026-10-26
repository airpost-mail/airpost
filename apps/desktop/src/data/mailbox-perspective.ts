/* eslint global-require: 0 */
/* eslint no-use-before-define: 0 */
import _ from 'underscore';

import { localized } from '../intl/localize';
import * as Utils from './models/utils';
import { TaskFactory } from './tasks/factory';
import { AccountStore } from './stores/account';
import CategoryStore from './stores/category';
import DatabaseStore from './database/store';
import OutboxStore from './stores/outbox';
import ThreadCountsStore from './stores/thread-counts';
import FolderSyncProgressStore from './stores/folder-sync-progress';
import { MutableQuerySubscription } from './query/mutable-subscription';
import UnreadQuerySubscription from './query/unread-subscription';
import { Thread } from './models/thread';
import { Category } from './models/category';
import { Label } from './models/label';
import { Folder } from './models/folder';
import { Task } from './tasks/task';
import {
  IntentTask,
  describeLabel,
  describeMove,
  perspectiveParams,
  withThreadCount,
} from './tasks/intent';
import { Method } from '../engine/protocol';
import type { Account } from './models/account';
import * as Actions from './actions';
import { QuerySubscription } from './query/subscription';
import { lazyRequire } from './lazy-require';

// Loaded on first use: these import this module.
const WorkspaceStore = (): typeof import('./stores/workspace').default =>
  require('./stores/workspace').default;
const FocusedPerspectiveStore = lazyRequire(
  (): typeof import('./stores/focused-perspective').default =>
    require('./stores/focused-perspective').default
);

// A shared role of null (mixed or no categories) is never one of `roles`.
function roleIsOneOf(role: string | null, roles: string[]) {
  return role !== null && roles.includes(role);
}

// The categories a serialized perspective carries.
function parseCategories(serializedCategories: string | undefined) {
  if (serializedCategories === undefined) {
    throw new Error('the perspective has no serializedCategories');
  }
  const json: unknown[] = JSON.parse(serializedCategories);
  return json.map(Utils.convertToModel) as (Category | null)[];
}

// This is a class cluster. Subclasses are not for external use!
// https://developer.apple.com/library/ios/documentation/General/Conceptual/CocoaEncyclopedia/ClassClusters/ClassClusters.html

export class MailboxPerspective {
  // Factory Methods
  static forNothing() {
    return new EmptyMailboxPerspective();
  }

  static forDrafts(accountsOrIds: string[]) {
    return new DraftsMailboxPerspective(accountsOrIds);
  }

  static forCategory(category: Category | null | undefined) {
    return category ? new CategoryMailboxPerspective([category]) : this.forNothing();
  }

  static forCategories(categories: (Category | null | undefined)[]) {
    const valid = categories.filter((c): c is Category => !!c);
    return valid.length > 0 ? new CategoryMailboxPerspective(valid) : this.forNothing();
  }

  static forStandardCategories(
    accountsOrIds: Account | string | (Account | string)[],
    ...names: string[]
  ) {
    const categories = CategoryStore.getCategoriesWithRoles(accountsOrIds, ...names);
    return this.forCategories(categories);
  }

  static forStarred(accountsOrIds: string[]) {
    return new StarredMailboxPerspective(accountsOrIds);
  }

  static forUnread(categories: Category[]) {
    return categories.length > 0 ? new UnreadMailboxPerspective(categories) : this.forNothing();
  }

  static forInbox(accountsOrIds: Account | string | (Account | string)[]) {
    return this.forStandardCategories(accountsOrIds, 'inbox');
  }

  static fromJSON(json: { type: string; serializedCategories?: string; accountIds: string[] }) {
    try {
      if (json.type === CategoryMailboxPerspective.name) {
        return this.forCategories(parseCategories(json.serializedCategories));
      }
      if (json.type === UnreadMailboxPerspective.name) {
        // A null here throws in the constructor, so the restore fails as before.
        return this.forUnread(parseCategories(json.serializedCategories) as Category[]);
      }
      if (json.type === StarredMailboxPerspective.name) {
        return this.forStarred(json.accountIds);
      }
      if (json.type === DraftsMailboxPerspective.name) {
        return this.forDrafts(json.accountIds);
      }
      return this.forInbox(json.accountIds);
    } catch (error) {
      AppEnv.reportError(new Error(`Could not restore mailbox perspective: ${error}`));
      return null;
    }
  }

  // Instance Methods

  accountIds: string[];
  // Every perspective but the empty one has a name and an icon.
  name?: string;
  iconName?: string;
  _categoriesSharedRole?: string | null;

  constructor(accountIds: string[]) {
    this.accountIds = accountIds;
    if (
      !(accountIds instanceof Array) ||
      !accountIds.every((aid) => typeof aid === 'string' || typeof aid === 'number')
    ) {
      throw new Error(`${this.constructor.name}: You must provide an array of string "accountIds"`);
    }
  }

  toJSON() {
    return { accountIds: this.accountIds, type: this.constructor.name };
  }

  isEqual(other: MailboxPerspective) {
    if (!other || this.constructor !== other.constructor) {
      return false;
    }
    if (other.name !== this.name) {
      return false;
    }
    if (!_.isEqual(this.accountIds, other.accountIds)) {
      return false;
    }
    return true;
  }

  isInbox() {
    return this.categoriesSharedRole() === 'inbox';
  }

  isSent() {
    return this.categoriesSharedRole() === 'sent';
  }

  isTrash() {
    return this.categoriesSharedRole() === 'trash';
  }

  isArchive() {
    return false;
  }

  emptyMessage(): React.ReactNode {
    return localized('No Messages');
  }

  categories(): Category[] {
    return [];
  }

  sheet() {
    const { Sheet } = WorkspaceStore();
    return Sheet && Sheet.Threads;
  }

  // overwritten in CategoryMailboxPerspective
  hasSyncingCategories(): boolean {
    return false;
  }

  categoriesSharedRole(): string | null {
    this._categoriesSharedRole =
      this._categoriesSharedRole || Category.categoriesSharedRole(this.categories());
    return this._categoriesSharedRole;
  }

  // The folder this perspective shows for an account, as `ChangeFolderTask.sourceFolderIds`,
  // so a move takes only the copies the user is looking at. Labels are not placements and
  // perspectives without a category (search, starred) have no copy to point at; both leave
  // the choice to the engine.
  sourceFolderIdsForAccount(accountId: string): string[] {
    const cat = this.categories().find((c) => c.accountId === accountId);
    return cat instanceof Folder ? [cat.id] : [];
  }

  category(): Category | null {
    return this.categories().length === 1 ? this.categories()[0] : null;
  }

  // Null for a perspective that shows no thread list (drafts).
  threads(): QuerySubscription<Thread> | null {
    throw new Error('threads: Not implemented in base class.');
  }

  unreadCount(): number {
    return 0;
  }

  // Public:
  // - accountIds {Array} Array of unique account ids associated with the threads
  // that want to be included in this perspective
  //
  // Returns true if the accountIds are part of the current ids, or false
  // otherwise. This means that it checks if I am attempting to move threads
  // between the same set of accounts:
  //
  // E.g.:
  // perpective = Starred for accountIds: a1, a2
  // thread1 has accountId a3
  // thread2 has accountId a2
  //
  // perspective.canReceiveThreadsFromAccountIds([a2, a3]) -> false -> I cant move those threads to Starred
  // perspective.canReceiveThreadsFromAccountIds([a2]) -> true -> I can move that thread to Starred
  canReceiveThreadsFromAccountIds(accountIds: string[] | null | undefined): boolean {
    if (!accountIds || accountIds.length === 0) {
      return false;
    }
    const areIncomingIdsInCurrent = _.difference(accountIds, this.accountIds).length === 0;
    return areIncomingIdsInCurrent;
  }

  receiveThreadIds(threadIds: Array<Thread | string>) {
    DatabaseStore.modelify<Thread>(Thread, threadIds).then((threads) => {
      const tasks = TaskFactory.tasksForThreadsByAccountId(threads, (accountThreads, accountId) => {
        return this.actionsForReceivingThreads(accountThreads, accountId);
      });
      if (tasks.length > 0) {
        Actions.queueTasks(tasks);
      }
    });
  }

  actionsForReceivingThreads(threads: Thread[], accountId: string): Task | Task[] | null {
    // eslint-disable-line
    throw new Error('actionsForReceivingThreads: Not implemented in base class.');
  }

  canArchiveThreads(threads: Thread[]) {
    if (this.isArchive()) {
      return false;
    }
    const accounts = AccountStore.accountsForItems(threads);
    return accounts.every((acc) => acc.canArchiveThreads());
  }

  canTrashThreads(threads: Thread[]) {
    return this.canMoveThreadsTo(threads, 'trash');
  }

  canMoveThreadsTo(threads: Thread[], standardCategoryName: string) {
    if (this.categoriesSharedRole() === standardCategoryName) {
      return false;
    }
    return AccountStore.accountsForItems(threads).every(
      (acc) => CategoryStore.getCategoryByRole(acc, standardCategoryName) !== null
    );
  }

  // What removing `threads` from this list does, for the swipe's look: null when it does
  // nothing. The engine decides the rest (`threads.removeFromView`).
  removalKind(threads: Thread[]): 'trash' | 'archive' | 'unstar' | null {
    return null;
  }

  tasksForRemovingItems(threads: Thread[], source?: string): Task[] {
    if (!(threads instanceof Array)) {
      throw new Error('tasksForRemovingItems: you must pass an array of threads or thread ids');
    }
    const kind = threads.length ? this.removalKind(threads) : null;
    if (!kind) {
      return [];
    }
    const preferTrash = !!AppEnv.config.get('core.reading.backspaceDelete');
    const n = threads.length;
    const description =
      kind === 'unstar'
        ? n > 1
          ? localized('Unstarred %@ threads', n)
          : localized('Unstarred')
        : withThreadCount(kind === 'trash' ? localized(`Trashed %@`) : localized(`Archived %@`), n);
    return [
      IntentTask.forThreads(
        Method.ThreadsRemoveFromView,
        threads,
        { perspective: perspectiveParams(this) || {}, ...(preferTrash ? { preferTrash } : {}) },
        source || 'Removed from list',
        description
      ),
    ];
  }
}

class DraftsMailboxPerspective extends MailboxPerspective {
  name = localized('Drafts');
  iconName = 'drafts.png';
  drafts = true; // The DraftListStore looks for this

  threads() {
    return null;
  }

  unreadCount() {
    let count = 0;
    for (const aid of this.accountIds) {
      count += OutboxStore.itemsForAccount(aid).length;
    }
    return count;
  }

  canReceiveThreadsFromAccountIds() {
    return false;
  }

  sheet() {
    const { Sheet } = WorkspaceStore();
    return Sheet && Sheet.Drafts;
  }
}

class StarredMailboxPerspective extends MailboxPerspective {
  starred = true;
  name = localized('Starred');
  iconName = 'starred.png';

  threads() {
    const query = DatabaseStore.findAll<Thread>(Thread)
      .where([Thread.attributes.starred.equal(true), Thread.attributes.inAllMail.equal(true)])
      .limit(0);

    // Adding a "account_id IN (a,b,c)" clause to our query can result in a full
    // table scan. Don't add the where clause if we know we want results from all.
    if (this.accountIds.length < AccountStore.accounts().length) {
      query.where(Thread.attributes.accountId.in(this.accountIds));
    }

    return new MutableQuerySubscription<Thread>(query, {
      emitResultSet: true,
      updateOnSeparateThread: true,
    });
  }

  canReceiveThreadsFromAccountIds(threads: string[]) {
    return super.canReceiveThreadsFromAccountIds(threads);
  }

  actionsForReceivingThreads(threads: Thread[], accountId: string) {
    return IntentTask.forThreads(
      Method.ThreadsStar,
      threads,
      { starred: true },
      'Dragged Into List'
    );
  }

  removalKind(threads: Thread[]) {
    return 'unstar' as const;
  }
}

class EmptyMailboxPerspective extends MailboxPerspective {
  constructor() {
    super([]);
  }

  threads() {
    // We need a Thread query that will not return any results and take no time.
    // We use lastMessageReceivedTimestamp because it is the first column on an
    // index so this returns zero items nearly instantly. In the future, we might
    // want to make a Query.forNothing() to go along with MailboxPerspective.forNothing()
    const query = DatabaseStore.findAll<Thread>(Thread)
      .where({ lastMessageReceivedTimestamp: -1 })
      .limit(0);
    return new MutableQuerySubscription<Thread>(query, {
      emitResultSet: true,
      updateOnSeparateThread: true,
    });
  }

  canReceiveThreadsFromAccountIds() {
    return false;
  }
}

class CategoryMailboxPerspective extends MailboxPerspective {
  _categories: Category[];

  constructor(_categories: Category[]) {
    super([...new Set(_categories.map((c) => c.accountId))]);
    this._categories = _categories;

    if (this._categories.length === 0) {
      throw new Error('CategoryMailboxPerspective: You must provide at least one category.');
    }

    // Note: We pick the display name and icon assuming that you won't create a
    // perspective with Inbox and Sent or anything crazy like that... todo?
    this.name = this._categories[0].displayName;
    if (this._categories[0].role) {
      this.iconName = `${this._categories[0].role}.png`;
    } else {
      this.iconName = this._categories[0] instanceof Label ? 'label.png' : 'folder.png';
    }
  }

  toJSON() {
    const json: any = super.toJSON();
    json.serializedCategories = JSON.stringify(this._categories);
    return json;
  }

  isEqual(other: MailboxPerspective) {
    return (
      super.isEqual(other) &&
      _.isEqual(
        this.categories().map((c) => c.id),
        other.categories().map((c) => c.id)
      )
    );
  }

  threads(): MutableQuerySubscription<Thread> {
    const query = DatabaseStore.findAll<Thread>(Thread)
      .where([Thread.attributes.categories.containsAny(this.categories().map((c) => c.id))])
      .limit(0);

    if (this.isSent()) {
      query.order(Thread.attributes.lastMessageSentTimestamp.descending());
    }

    if (!roleIsOneOf(this.categoriesSharedRole(), ['spam', 'trash'])) {
      query.where({ inAllMail: true });
    }

    if (this._categories.length > 1 && this.accountIds.length < this._categories.length) {
      // The user has multiple categories in the same account selected, which
      // means our result set could contain multiple copies of the same threads
      // (since we do an inner join) and we need SELECT DISTINCT. Note that this
      // can be /much/ slower and we shouldn't do it if we know we don't need it.
      query.distinct();
    }

    return new MutableQuerySubscription<Thread>(query, {
      emitResultSet: true,
      updateOnSeparateThread: true,
    });
  }

  unreadCount() {
    let sum = 0;
    for (const cat of this._categories) {
      // An unknown count adds nothing (null reads as 0).
      sum += ThreadCountsStore.unreadCountForCategoryId(cat.id) ?? 0;
    }
    return sum;
  }

  categories() {
    return this._categories;
  }

  hasSyncingCategories() {
    return this._categories.some((cat) => {
      const representedFolder =
        cat instanceof Folder ? cat : CategoryStore.getAllMailCategory(cat.accountId);
      return (
        representedFolder &&
        FolderSyncProgressStore.isSyncingAccount(cat.accountId, representedFolder.path)
      );
    });
  }

  isArchive() {
    return this._categories.every((cat) => cat.isArchive());
  }

  canReceiveThreadsFromAccountIds(threads: string[]) {
    return (
      super.canReceiveThreadsFromAccountIds(threads) &&
      !this._categories.some((c) => c.isLockedCategory())
    );
  }

  // Each drop is the engine's intents; dropping from a folder onto a Gmail label is two (to
  // All Mail, then the label), so two undo entries.
  actionsForReceivingThreads(threads: Thread[], accountId: string): Task[] {
    const current = FocusedPerspectiveStore().current();

    // This assumes that the we don't have more than one category per
    // accountId attached to this perspective
    if (roleIsOneOf(current.categoriesSharedRole(), Category.LockedRoles)) {
      return [];
    }

    // Threads are only dropped here from this perspective's accounts.
    const myCat = this.categories().find((c) => c.accountId === accountId)!;
    const currentCat = current.categories().find((c) => c.accountId === accountId);

    // Don't drag and drop on ourselves
    // NOTE: currentCat can be nil in case of SearchPerspective
    if (currentCat && myCat.id === currentCat.id) {
      return [];
    }

    const source = 'Dragged into list';
    // A move takes only the copies the user is looking at; a copy of the same message in
    // Sent (or any other folder) stays put.
    const perspective = perspectiveParams(current);
    const n = threads.length;
    const move = (to: Category) =>
      IntentTask.forThreads(
        Method.ThreadsMove,
        threads,
        { to: to.id, perspective },
        source,
        describeMove(n, to.displayName)
      );
    const label = (add: Category[], remove: Category[]) =>
      IntentTask.forThreads(
        Method.ThreadsLabel,
        threads,
        { add: add.map((c) => c.id), remove: remove.map((c) => c.id) },
        source,
        add.length
          ? describeLabel(n, add[0].displayName, true)
          : describeLabel(n, remove[0].displayName, false)
      );

    if (myCat.role === 'all' && currentCat && currentCat instanceof Label) {
      // dragging from a label into All Mail? Make this an "archive" by removing the
      // label. Otherwise (Since labels are subsets of All Mail) it'd have no effect.
      return [label([], [currentCat])];
    }
    if (myCat instanceof Folder) {
      // dragging to a folder like spam, trash or any IMAP folder? Just change the folder.
      return [move(myCat)];
    }
    if (myCat instanceof Label && currentCat && currentCat instanceof Folder) {
      // dragging from trash or spam into a label? We need to both apply the label and
      // move to the "All Mail" folder.
      const all = CategoryStore.getCategoryByRole(accountId, 'all');
      return [...(all ? [move(all)] : []), label([myCat], [])];
    }
    // label to label
    return [label([myCat], currentCat ? [currentCat] : [])];
  }

  // From Archive: to the trash. From Spam or Trash: nothing. From another folder or a label:
  // the account's preferred removal (`Account.preferredRemovalDestination`).
  removalKind(threads: Thread[]) {
    if (this.isArchive()) {
      return 'trash' as const;
    }
    if (roleIsOneOf(this.categoriesSharedRole(), ['spam', 'trash'])) {
      return null;
    }
    const account = threads.length ? AccountStore.accountForId(threads[0].accountId) : null;
    const preferred = account && account.preferredRemovalDestination();
    if (!preferred) return null;
    return preferred.role === 'trash' ? ('trash' as const) : ('archive' as const);
  }
}

class UnreadMailboxPerspective extends CategoryMailboxPerspective {
  unread = true;
  name = localized('Unread');
  iconName = 'unread.png';

  threads(): MutableQuerySubscription<Thread> {
    return new UnreadQuerySubscription(this.categories().map((c) => c.id));
  }

  unreadCount() {
    return 0;
  }

  actionsForReceivingThreads(threads: Thread[], accountId: string) {
    const tasks = super.actionsForReceivingThreads(threads, accountId);
    tasks.push(
      IntentTask.forThreads(Method.ThreadsMarkRead, threads, { unread: true }, 'Dragged Into List')
    );
    return tasks;
  }
}
