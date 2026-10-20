import { ipcRenderer } from 'electron';
import _ from 'underscore';
import BaseStore from 'mailspring-store';
import WorkspaceStore from './workspace';
import FocusedPerspectiveStore from './focused-perspective';
import * as Actions from '../actions';

const MAIN_TAB_ITEM_ID = 'General';

/** The sidebar group a tab is listed under. `plugins` holds tabs that name no group. */
export type PreferencesTabGroup = 'server' | 'app' | 'mail' | 'connected' | 'plugins' | 'advanced';

export class PreferencesUIStoreTab {
  order: number;
  tabId: string;
  displayName: string;
  componentClassFn: () => any;
  group: PreferencesTabGroup;
  /** Extra words the sidebar search matches, besides the display name. */
  keywords: string[];

  constructor(opts: {
    tabId: string;
    displayName: string;
    componentClassFn: () => any;
    order?: number;
    group?: PreferencesTabGroup;
    keywords?: string[];
  }) {
    this.tabId = opts.tabId;
    this.componentClassFn = opts.componentClassFn;
    this.displayName = opts.displayName;
    this.order = opts.order || Infinity;
    this.group = opts.group || 'plugins';
    this.keywords = opts.keywords || [];
  }
}

class _PreferencesUIStore extends BaseStore {
  _tabs: PreferencesUIStoreTab[] = [];
  _selection = {
    tabId: null,
    accountId: null,
  };

  constructor() {
    super();

    const perspective = FocusedPerspectiveStore.current();
    this._selection = {
      tabId: null,
      accountId: perspective['account'] ? perspective['account'].id : null,
    };

    this.setupListeners();
  }

  get TabItem() {
    return PreferencesUIStoreTab;
  }

  _triggerDebounced = _.debounce(() => this.trigger(), 20);

  setupListeners() {
    if (AppEnv.isMainWindow()) {
      this.listenTo(Actions.openPreferences, this.openPreferences);
      ipcRenderer.on('open-preferences', this.openPreferences);

      this.listenTo(Actions.switchPreferencesTab, this.switchPreferencesTab);
    }

    AppEnv.commands.add(document.body, 'core:show-keybindings', () => {
      Actions.openPreferences();
      Actions.switchPreferencesTab('Shortcuts');
    });
  }

  tabs() {
    return this._tabs;
  }

  selection() {
    return this._selection;
  }

  openPreferences = () => {
    ipcRenderer.send('command', 'application:show-main-window');
    if (WorkspaceStore.topSheet() !== WorkspaceStore.Sheet.Preferences) {
      Actions.pushSheet(WorkspaceStore.Sheet.Preferences);
    }
  };

  switchPreferencesTab = (tabId: string, options: { accountId?: string } = {}) => {
    this._selection.tabId = tabId;
    if (options.accountId) {
      this._selection.accountId = options.accountId;
    }
    this.trigger();
  };

  /*
  Public: Register a new top-level section to preferences

  - `tabItem` a `PreferencesUIStore.TabItem` object
    schema definitions on the PreferencesUIStore.Section.MySectionId
    - `tabId` A unique name to access the Section by
    - `displayName` The display name. This may go through i18n.
    - `component` The Preference section's React Component.
    - `group` The sidebar group: `app`, `mail`, `connected`, `advanced`, or `plugins` (default)
    - `keywords` Extra words the sidebar search matches

  Most Preference sections include an area where a {PreferencesForm} is
  rendered. This is a type of {GeneratedForm} that uses the schema passed
  into {PreferencesUIStore::registerPreferences}

  */
  registerPreferencesTab = (tabItem: PreferencesUIStoreTab) => {
    this._tabs.push(tabItem);
    this._tabs.sort((a, b) => (a.order === b.order ? 0 : a.order < b.order ? -1 : 1));
    if (tabItem.tabId === MAIN_TAB_ITEM_ID) {
      this._selection.tabId = tabItem.tabId;
    }
    this._triggerDebounced();
  };

  unregisterPreferencesTab = (tabItemOrId: string | PreferencesUIStoreTab) => {
    this._tabs = this._tabs.filter((s) => s.tabId !== tabItemOrId && s !== tabItemOrId);
    this._triggerDebounced();
  };
}

export const PreferencesUIStore = new _PreferencesUIStore();
