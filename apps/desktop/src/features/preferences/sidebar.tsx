import React from 'react';
import classNames from 'classnames';
import { RetinaImg, SheetTitleBar } from 'mailspring-component-kit';
import {
  Account,
  AccountStore,
  Actions,
  PreferencesUIStoreTab,
  localized,
} from 'mailspring-exports';
import { PreferencesIcon } from './icon';
import { TextField } from '../../components/kit/text-field';

type Group = PreferencesUIStoreTab['group'];

const GROUPS: { id: Group; label: () => string | null }[] = [
  { id: 'app', label: () => localized('App') },
  { id: 'mail', label: () => localized('Mail') },
  { id: 'connected', label: () => localized('Connected') },
  { id: 'plugins', label: () => localized('From plugins') },
  { id: 'advanced', label: () => null },
];

interface PreferencesSidebarProps {
  tabs: PreferencesUIStoreTab[];
  selection: { tabId: string; accountId: string };
}

function matches(tab: PreferencesUIStoreTab, query: string) {
  if (!query) return true;
  const haystack = [tab.displayName, ...tab.keywords].join(' ').toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .every((word) => haystack.includes(word));
}

export default class PreferencesSidebar extends React.Component<
  PreferencesSidebarProps,
  { query: string; accountErrors: number }
> {
  static displayName = 'PreferencesSidebar';

  state = { query: '', accountErrors: this._accountErrors() };
  _unlisten: () => void;
  _navRef = React.createRef<HTMLDivElement>();

  componentDidMount() {
    this._unlisten = AccountStore.listen(() =>
      this.setState({ accountErrors: this._accountErrors() })
    );
  }

  componentWillUnmount() {
    this._unlisten?.();
  }

  _accountErrors() {
    return AccountStore.accounts().filter(
      (a: Account) => a.syncState && a.syncState !== Account.SYNC_STATE_OK
    ).length;
  }

  _visibleTabs() {
    return this.props.tabs.filter((t) => matches(t, this.state.query));
  }

  _onSearchKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Escape' && this.state.query) {
      e.stopPropagation();
      this.setState({ query: '' });
    } else if (e.key === 'Enter') {
      const first = this._visibleTabs().find((t) => t.group !== 'server');
      if (first) Actions.switchPreferencesTab(first.tabId);
    }
  };

  _onNavKeyDown = (e: React.KeyboardEvent) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const items = Array.from(
      this._navRef.current?.querySelectorAll<HTMLElement>('[data-tab-id]') || []
    );
    const idx = items.findIndex((el) => el.dataset.tabId === this.props.selection.tabId);
    const next = items[(idx + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length];
    if (!next) return;
    e.preventDefault();
    Actions.switchPreferencesTab(next.dataset.tabId);
    next.focus();
  };

  _renderItem(tab: PreferencesUIStoreTab) {
    const active = tab.tabId === this.props.selection.tabId;
    const badge =
      tab.tabId === 'Accounts' && this.state.accountErrors > 0 ? (
        <span
          className="preferences-sidebar-badge"
          aria-label={localized('%@ accounts need attention', `${this.state.accountErrors}`)}
        >
          {this.state.accountErrors}
        </span>
      ) : null;
    return (
      <div
        key={tab.tabId}
        data-tab-id={tab.tabId}
        role="tab"
        aria-selected={active}
        tabIndex={active ? 0 : -1}
        className={classNames('preferences-sidebar-item', { active })}
        onClick={() => Actions.switchPreferencesTab(tab.tabId)}
      >
        <PreferencesIcon tabId={tab.tabId} />
        <span className="name">{tab.displayName}</span>
        {badge}
      </div>
    );
  }

  _renderServer(tab: PreferencesUIStoreTab) {
    const active = tab.tabId === this.props.selection.tabId;
    return (
      <div
        key={tab.tabId}
        data-tab-id={tab.tabId}
        role="tab"
        aria-selected={active}
        tabIndex={active ? 0 : -1}
        className={classNames('preferences-sidebar-server', { active })}
        onClick={() => Actions.switchPreferencesTab(tab.tabId)}
      >
        <span className="bird">
          <RetinaImg
            name="tern.png"
            mode={RetinaImg.Mode.ContentPreserve}
            style={{ width: 22, height: 22 }}
          />
        </span>
        <span>
          <b>{tab.displayName}</b>
          <small>{localized('Not set up')}</small>
        </span>
      </div>
    );
  }

  render() {
    const visible = this._visibleTabs();
    const server = visible.filter((t) => t.group === 'server');
    return (
      <div className="preferences-sidebar">
        <SheetTitleBar side="start" />
        <div className="preferences-sidebar-search">
          <TextField
            type="search"
            placeholder={localized('Search settings')}
            aria-label={localized('Search settings')}
            value={this.state.query}
            onChange={(e) => this.setState({ query: e.target.value })}
            onKeyDown={this._onSearchKeyDown}
          />
        </div>
        <div
          className="preferences-sidebar-nav"
          role="tablist"
          aria-orientation="vertical"
          aria-label={localized('Preferences')}
          ref={this._navRef}
          onKeyDown={this._onNavKeyDown}
        >
          {server.map((t) => this._renderServer(t))}
          {GROUPS.map(({ id, label }) => {
            const tabs = visible.filter((t) => t.group === id);
            if (!tabs.length) return null;
            const text = label();
            return (
              <div key={id} className={`preferences-sidebar-group group-${id}`}>
                {text ? (
                  <div className="preferences-sidebar-label">{text}</div>
                ) : (
                  <div className="preferences-sidebar-separator" />
                )}
                {tabs.map((t) => this._renderItem(t))}
              </div>
            );
          })}
          {visible.length === 0 && (
            <div className="preferences-sidebar-empty">{localized('No matching settings')}</div>
          )}
        </div>
      </div>
    );
  }
}
