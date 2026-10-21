import React from 'react';
import ReactDOM from 'react-dom';
import { Flexbox } from '../../components/flexbox';
import { ScrollRegion } from '../../components/scroll-region';
import { KeyCommandsRegion } from '../../components/key-commands-region';
import ListensToFluxStore from '../../components/decorators/listens-to-flux-store';
import { ConfigPropContainer } from '../../components/config-prop-container';
import { SheetTitleBar } from '../../components/sheets/toolbar';
import { PreferencesUIStore } from '../../data/stores/preferences-ui';
import PreferencesSidebar from './sidebar';

const stopPropagation = (e: CustomEvent) => {
  e.stopPropagation();
};

class PreferencesRoot extends React.Component<{ tab: any; tabs: any[]; selection: any }> {
  static displayName = 'PreferencesRoot';

  // This prevents some basic commands from propagating to the threads list and
  // producing unexpected results

  // TODO This is a partial/temporary solution and should go away when we do the
  // Keymap/Commands/Menu refactor
  _localHandlers = {
    'core:next-item': stopPropagation,
    'core:previous-item': stopPropagation,
    'core:select-up': stopPropagation,
    'core:select-down': stopPropagation,
    'core:select-item': stopPropagation,
    'core:messages-page-up': stopPropagation,
    'core:messages-page-down': stopPropagation,
    'core:list-page-up': stopPropagation,
    'core:list-page-down': stopPropagation,
    'core:remove-from-view': stopPropagation,
    'core:gmail-remove-from-view': stopPropagation,
    'core:remove-and-previous': stopPropagation,
    'core:remove-and-next': stopPropagation,
    'core:archive-item': stopPropagation,
    'core:delete-item': stopPropagation,
    'core:print-thread': stopPropagation,
  };

  componentDidMount() {
    (ReactDOM.findDOMNode(this) as HTMLElement).focus();
    this._focusContent();
  }

  componentDidUpdate(oldProps: { tab: any; tabs: any[]; selection: any }) {
    if (oldProps.tab !== this.props.tab) {
      const scrollRegion = document.querySelector('.preferences-content .scroll-region-content');
      if (scrollRegion) scrollRegion.scrollTop = 0;
      this._focusContent();
    }
  }

  // Focus the selected sidebar item, so the arrow keys move between tabs and Tab reaches the
  // page. Focusing the page's first control instead lit up its focus ring on every click.
  _focusContent() {
    const root = ReactDOM.findDOMNode(this) as HTMLElement;
    root?.querySelector<HTMLElement>('.preferences-sidebar [aria-selected="true"]')?.focus();
  }

  render() {
    const { tab, selection, tabs } = this.props;
    const TabComponent = tab && tab.componentClassFn();

    // The wrapper takes focus on mount and on clicks in blank space, but is not a Tab stop.
    return (
      <KeyCommandsRegion
        className="preferences-wrap"
        tabIndex={-1}
        localHandlers={this._localHandlers}
      >
        <Flexbox direction="row">
          <PreferencesSidebar tabs={tabs} selection={selection} />
          <div className="preferences-main">
            <SheetTitleBar side="end" />
            <ScrollRegion className="preferences-content">
              {/* ConfigPropContainer clones its child, so it needs one. */}
              {tab ? (
                <ConfigPropContainer>
                  <TabComponent accountId={selection.accountId} />
                </ConfigPropContainer>
              ) : null}
            </ScrollRegion>
          </div>
        </Flexbox>
      </KeyCommandsRegion>
    );
  }
}

export default ListensToFluxStore(PreferencesRoot, {
  stores: [PreferencesUIStore],
  getStateFromStores() {
    const tabs = PreferencesUIStore.tabs();
    const selection = PreferencesUIStore.selection();
    const tab = tabs.find((t) => t.tabId === selection.tabId);
    return { tabs, selection, tab };
  },
});
