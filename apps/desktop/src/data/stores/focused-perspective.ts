import _ from 'underscore';
import { ipcRenderer } from 'electron';
import BaseStore from './base';
import { AccountStore } from './account';
import WorkspaceStore from './workspace';
import { MailboxPerspective } from '../mailbox-perspective';
import CategoryStore from './category';
import * as Actions from '../actions';
import type { Account } from '../models/account';
import type { Folder } from '../models/folder';
import type { Label } from '../models/label';

class FocusedPerspectiveStore extends BaseStore {
  _current: MailboxPerspective = MailboxPerspective.forNothing();
  _initialized = false;

  constructor() {
    super();

    this.listenTo(CategoryStore, this._onCategoryStoreChanged);
    this.listenTo(Actions.focusMailboxPerspective, this._onFocusPerspective);
    this.listenTo(Actions.focusDefaultMailboxPerspectiveForAccounts, this._onFocusDefault);
    this.listenTo(Actions.ensureCategoryIsFocused, this._onEnsureCategoryIsFocused);
    this._listenToCommands();

    // Sent by the main process for application:show-mail.
    if (AppEnv.isMainWindow()) {
      ipcRenderer.on('show-mail', () => this.showMail());
    }
  }

  /** Whether the main window shows mail rather than another section, such as the calendar. */
  isShowingMail() {
    return WorkspaceStore.rootSheet() === this._current.sheet();
  }

  /** Leaves another section of the main window for the mailbox that was last showing. */
  showMail() {
    if (!this.isShowingMail()) {
      Actions.selectRootSheet(this._current.sheet());
    } else if (WorkspaceStore.topSheet() === WorkspaceStore.Sheet.Preferences) {
      Actions.popSheet();
    }
  }

  current(): MailboxPerspective {
    return this._current;
  }

  sidebarAccountIds() {
    let ids: string[] = AppEnv.savedState.sidebarAccountIds;
    if (!ids || !ids.length || !ids.every((id) => AccountStore.accountForId(id))) {
      ids = AppEnv.savedState.sidebarAccountIds = AccountStore.accountIds();
    }

    // Always defer to the AccountStore for the desired order of accounts in
    // the sidebar - users can re-arrange them!
    const order = AccountStore.accountIds();
    ids = ids.sort((a, b) => order.indexOf(a) - order.indexOf(b));

    return ids;
  }

  _listenToCommands() {
    AppEnv.commands.add(document.body, {
      'navigation:go-to-inbox': () => this._setPerspectiveByName('inbox'),
      'navigation:go-to-sent': () => this._setPerspectiveByName('sent'),
      'navigation:go-to-unread': () => this._setUnreadPerspective(),
      'navigation:go-to-starred': () =>
        this._setPerspective(MailboxPerspective.forStarred(this._current.accountIds)),
      'navigation:go-to-drafts': () =>
        this._setPerspective(MailboxPerspective.forDrafts(this._current.accountIds)),
      'navigation:go-to-all': () => {
        const categories = this._current.accountIds.map((id) =>
          CategoryStore.getArchiveCategory(id)
        );
        this._setPerspective(MailboxPerspective.forCategories(categories));
      },
      'navigation:go-to-contacts': () => {}, // TODO,
      'navigation:go-to-tasks': () => {}, // TODO,
      'navigation:go-to-label': () => {}, // TODO,
    });
  }

  _isValidAccountSet(ids: string[]) {
    const accountIds = AccountStore.accountIds();
    return ids.every((a) => accountIds.includes(a));
  }

  _isValidPerspective(perspective: MailboxPerspective) {
    // Ensure all the accountIds referenced in the perspective still exist
    if (!this._isValidAccountSet(perspective.accountIds)) {
      return false;
    }

    // Ensure all the categories referenced in the perspective still exist
    const categoriesStillExist = perspective.categories().every((c) => {
      return !!CategoryStore.byId(c.accountId, c.id);
    });
    if (!categoriesStillExist) {
      return false;
    }

    return true;
  }

  _initializeFromSavedState() {
    const json = AppEnv.savedState.perspective;
    let { sidebarAccountIds } = AppEnv.savedState;
    let perspective;

    if (json) {
      perspective = MailboxPerspective.fromJSON(json);
    }
    this._initialized = true;

    if (!perspective || !this._isValidPerspective(perspective)) {
      perspective = this._defaultPerspective();
      sidebarAccountIds = perspective.accountIds;
      this._initialized = false;
    }

    if (
      !sidebarAccountIds ||
      !this._isValidAccountSet(sidebarAccountIds) ||
      sidebarAccountIds.length < perspective.accountIds.length
    ) {
      sidebarAccountIds = perspective.accountIds;
      this._initialized = false;
    }

    this._setPerspective(perspective, sidebarAccountIds);
    if (this._initialized && this._unifiedSidebarIsMissingAccounts()) {
      this._setPerspective(this._defaultPerspective(), AccountStore.accountIds());
    }
  }

  // The sidebar shows either one account or all of them, so a saved multi-account
  // selection that lacks an account means the account was added since. Waits for the
  // new accounts' inboxes, so the unified inbox includes them.
  _unifiedSidebarIsMissingAccounts() {
    const ids: string[] = AppEnv.savedState.sidebarAccountIds || [];
    if (ids.length < 2) {
      return false;
    }
    const missing = AccountStore.accountIds().filter((id) => !ids.includes(id));
    return (
      missing.length > 0 &&
      missing.every((id) => !!CategoryStore.getCategoryByRole(id, 'inbox'))
    );
  }

  // Inbound Events
  _onCategoryStoreChanged = () => {
    if (!this._initialized) {
      this._initializeFromSavedState();
    } else if (this._unifiedSidebarIsMissingAccounts()) {
      this._setPerspective(this._defaultPerspective(), AccountStore.accountIds());
    } else if (!this._isValidPerspective(this._current)) {
      this._setPerspective(this._defaultPerspective(this._current.accountIds));
    } else {
      this._refreshRenamedCategories();
    }
  };

  // A renamed folder or label keeps its id, so the perspective stays valid but holds the
  // old path, which its name and the sidebar selection come from. Swapped in place, without
  // _setPerspective's sheet change; ThreadListStore keeps its list and the open thread for a
  // perspective with the same ids (ThreadListStore.perspectiveKey).
  _refreshRenamedCategories() {
    const json = this._current.toJSON() as {
      type: string;
      accountIds: string[];
      serializedCategories?: string;
    };
    if (!json.serializedCategories) {
      return;
    }
    const categories = this._current.categories();
    const fresh = categories.map((c) => CategoryStore.byId(c.accountId, c.id) || c);
    const renamed = fresh.some(
      (f, i) => f.path !== categories[i].path || f.role !== categories[i].role
    );
    if (!renamed) {
      return;
    }
    const next = MailboxPerspective.fromJSON({
      ...json,
      serializedCategories: JSON.stringify(fresh),
    });
    if (!next) {
      return;
    }
    this._current = next;
    AppEnv.savedState.perspective = next.toJSON();
    this.trigger();
  }

  _onFocusPerspective = (perspective: MailboxPerspective) => {
    // If looking at unified inbox, don't attempt to change the sidebar accounts
    const sidebarIsUnifiedInbox = this.sidebarAccountIds().length > 1;
    if (sidebarIsUnifiedInbox) {
      this._setPerspective(perspective);
    } else {
      this._setPerspective(perspective, perspective.accountIds);
    }
  };

  /*
   * Takes an optional array of `sidebarAccountIds`. By default, this method will
   * set the sidebarAccountIds to the perspective's accounts if no value is
   * provided
   */
  _onFocusDefault = (
    accountsOrIds: Account | string | (Account | string)[] | null | undefined,
    { sidebarAccountIds }: { sidebarAccountIds?: string[] } = {}
  ) => {
    if (!accountsOrIds) {
      return;
    }
    const perspective = this._defaultPerspective(accountsOrIds);
    this._setPerspective(perspective, sidebarAccountIds || perspective.accountIds);
  };

  _onEnsureCategoryIsFocused = (categoryName: string, accountIds: string[] = []) => {
    const ids = accountIds instanceof Array ? accountIds : [accountIds];
    const categories = ids.map((id) => CategoryStore.getCategoryByRole(id, categoryName));
    const perspective = MailboxPerspective.forCategories(categories);
    this._onFocusPerspective(perspective);
  };

  _defaultPerspective(
    accountsOrIds: Account | string | (Account | string)[] = AccountStore.accountIds()
  ) {
    const perspective = MailboxPerspective.forInbox(accountsOrIds);

    // If no account ids were selected, or the categories for these accounts have
    // not loaded yet, return forNothing(). This means that the next time the
    // CategoryStore triggers, we'll try again.
    if (perspective.categories().length === 0) {
      return MailboxPerspective.forNothing();
    }
    return perspective;
  }

  _setPerspective(perspective: MailboxPerspective, sidebarAccountIds?: string[]) {
    let shouldTrigger = false;

    if (!perspective.isEqual(this._current)) {
      AppEnv.savedState.perspective = perspective.toJSON();
      this._current = perspective;
      shouldTrigger = true;
    }

    const shouldSaveSidebarAccountIds =
      sidebarAccountIds &&
      !_.isEqual(AppEnv.savedState.sidebarAccountIds, sidebarAccountIds) &&
      this._initialized === true;
    if (shouldSaveSidebarAccountIds) {
      AppEnv.savedState.sidebarAccountIds = sidebarAccountIds;
      shouldTrigger = true;
    }

    if (shouldTrigger) {
      this.trigger();
    }

    const desired = perspective.sheet();

    // Always switch to the correct sheet and pop to root when perspective set
    if (desired && WorkspaceStore.rootSheet() !== desired) {
      Actions.selectRootSheet(desired);
    }

    const top = WorkspaceStore.topSheet();
    if (![WorkspaceStore.Sheet.Preferences, WorkspaceStore.Sheet.Threads].includes(top)) {
      Actions.popToRootSheet();
    }
  }

  _setUnreadPerspective() {
    const inboxes = this._current.accountIds
      .map((id) => CategoryStore.getCategoryByRole(id, 'inbox'))
      .filter((c): c is Folder | Label => !!c);
    if (inboxes.length === 0) {
      return;
    }
    this._setPerspective(MailboxPerspective.forUnread(inboxes));
  }

  _setPerspectiveByName(categoryName: string) {
    let categories = this._current.accountIds.map((id) =>
      CategoryStore.getCategoryByRole(id, categoryName)
    );
    categories = categories.filter(Boolean);
    if (categories.length === 0) {
      return;
    }
    this._setPerspective(MailboxPerspective.forCategories(categories));
  }
}

export default new FocusedPerspectiveStore();
