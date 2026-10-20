import React from 'react';
import {
  ScrollRegion,
  ListensToFluxStore,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from 'mailspring-component-kit';
import { localized, AccountStore, FocusedPerspectiveStore } from 'mailspring-exports';
import { ownKeys } from '../../../src/components/kit/keys';

import ActivityFeed from './feed/feed';
import ActivityReports from './dashboard/root';
import EngagementBoard from './engagement/board';
import TimespanSelector from './timespan-selector';
import { DEFAULT_TIMESPAN_ID, Timespan, timespanForId } from './timespan';
import * as ActivityActions from './actions';

export type ActivityTabId = 'feed' | 'engagement' | 'reports';

const TABS: { id: ActivityTabId; label: () => string }[] = [
  { id: 'feed', label: () => localized('Feed') },
  { id: 'engagement', label: () => localized('Engagement') },
  { id: 'reports', label: () => localized('Reports') },
];

// The keys the tab list acts on don't also reach the global keymap (core:move-left, …).
const ownTabKeys = ownKeys(['ArrowLeft', 'ArrowRight', 'Home', 'End', ' ', 'Enter']);

class Root extends React.Component<
  { accountIds: string[] },
  { tab: ActivityTabId; timespan: Timespan }
> {
  static displayName = 'ActivityRoot';

  _unlisten: () => void;

  constructor(props) {
    super(props);
    const saved = AppEnv.savedState.activityTab;
    this.state = {
      tab: TABS.some((t) => t.id === saved) ? saved : 'feed',
      timespan: timespanForId(DEFAULT_TIMESPAN_ID),
    };
  }

  componentDidMount() {
    this._unlisten = ActivityActions.selectTab.listen(this._onSelectTab);
  }

  componentWillUnmount() {
    this._unlisten();
  }

  _onSelectTab = (tab: ActivityTabId) => {
    AppEnv.savedState.activityTab = tab;
    AppEnv.saveWindowState();
    this.setState({ tab });
  };

  _onChangeTimespan = (timespanId: string) => {
    this.setState({ timespan: timespanForId(timespanId) });
  };

  render() {
    const { accountIds } = this.props;
    const { tab, timespan } = this.state;
    const account = AccountStore.accountForId(accountIds[0]);

    return (
      <div className="activity-dashboard">
        <div className="header">
          <div style={{ flex: 1 }}>
            <h2>{localized('Activity')}</h2>
            <div className="accounts">
              {accountIds.length > 1 ? localized('All Accounts') : account && account.label}
            </div>
          </div>
          <TimespanSelector timespan={timespan} onChange={this._onChangeTimespan} />
        </div>
        <Tabs
          className="activity-tabs-root"
          value={tab}
          onValueChange={(id) => this._onSelectTab(id as ActivityTabId)}
        >
          <TabsList
            className="activity-tabs hidden-on-web"
            aria-label={localized('Activity')}
            onKeyDown={ownTabKeys}
          >
            {TABS.map((t) => (
              <TabsTrigger key={t.id} value={t.id}>
                {t.label()}
              </TabsTrigger>
            ))}
          </TabsList>
          {/* Every panel stays mounted so Reports doesn't recompute on each switch;
              data-state="inactive" hides the others. */}
          <TabsContent value="feed" forceMount className="activity-tab-panel hidden-on-web">
            <ActivityFeed accountIds={accountIds} timespan={timespan} />
          </TabsContent>
          <TabsContent value="engagement" forceMount className="activity-tab-panel hidden-on-web">
            <EngagementBoard accountIds={accountIds} timespan={timespan} />
          </TabsContent>
          <TabsContent value="reports" forceMount className="activity-tab-panel reports">
            <ScrollRegion className="activity-reports-scroll">
              <ActivityReports accountIds={accountIds} timespan={timespan} />
            </ScrollRegion>
          </TabsContent>
        </Tabs>
      </div>
    );
  }
}

export default ListensToFluxStore(Root, {
  stores: [FocusedPerspectiveStore],
  getStateFromStores: (props) => {
    return {
      ...props,
      accountIds: FocusedPerspectiveStore.current().accountIds,
    };
  },
});
