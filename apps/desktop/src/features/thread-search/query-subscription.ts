import _ from 'underscore';
import * as Actions from '../../data/actions';
import { Thread } from '../../data/models/thread';
import DatabaseStore from '../../data/database/store';
import { SearchQueryParser } from '../../mail/search/parser';
import ComponentRegistry from '../../packages/component-registry';
import { MutableQuerySubscription } from '../../data/query/mutable-subscription';

class SearchQuerySubscription extends MutableQuerySubscription<Thread> {
  _searchQuery: string;
  _accountIds: string[];
  _connections: { end: () => void }[] = [];
  _extDisposables: { dispose: () => void }[] = [];
  _searching = false;

  constructor(searchQuery: string, accountIds: string[]) {
    super(null, { emitResultSet: true });
    this._searchQuery = searchQuery;
    this._accountIds = accountIds;

    _.defer(() => this.performSearch());
  }

  replaceRange = () => {
    // TODO
  };

  performSearch() {
    this._searching = true;
    this.performLocalSearch();
    this.performExtensionSearch();
  }

  performLocalSearch() {
    let dbQuery = DatabaseStore.findAll<Thread>(Thread);
    if (this._accountIds.length === 1) {
      dbQuery = dbQuery.where({ accountId: this._accountIds[0] });
    }

    try {
      const parsedQuery = SearchQueryParser.parse(this._searchQuery);
      dbQuery = dbQuery.structuredSearch(parsedQuery);
    } catch (e) {
      console.info('Failed to parse local search query, falling back to generic query', e);
      dbQuery = dbQuery.search(this._searchQuery);
    }
    dbQuery = dbQuery
      .background()
      .order(Thread.attributes.lastMessageReceivedTimestamp.descending())
      .limit(1000);

    this.replaceQuery(dbQuery);
  }

  _createResultAndTrigger() {
    super._createResultAndTrigger();
    if (this._searching) {
      this._searching = false;
      Actions.searchCompleted();
    }
  }

  _addThreadIdsToSearch(ids: string[] = []) {
    const set = this._set;
    let searchIds = ids;
    if (set && set.ids().length > 0) {
      const currentResultIds = set.ids();
      searchIds = [...new Set(currentResultIds.concat(ids))];
    }
    const dbQuery = DatabaseStore.findAll<Thread>(Thread)
      .where({ id: searchIds })
      .order(Thread.attributes.lastMessageReceivedTimestamp.descending());
    this.replaceQuery(dbQuery);
  }

  performRemoteSearch() {
    // TODO: Perform IMAP search here.
    //
    // This is temporarily disabled because we support Gmail's
    // advanced syntax locally (eg: in: inbox, is:unread), and
    // search message bodies, so local search is pretty much
    // good enough for v1. Come back and implement this soon!
    //
  }

  performExtensionSearch() {
    const searchExtensions = ComponentRegistry.findComponentsMatching({
      role: 'SearchBarResults',
    });

    this._extDisposables = searchExtensions.map((ext) => {
      return ext.observeThreadIdsForQuery(this._searchQuery).subscribe((ids = []) => {
        const allIds = ids.flat().filter(Boolean);
        if (allIds.length === 0) return;
        this._addThreadIdsToSearch(allIds);
      });
    });
  }

  onLastCallbackRemoved() {
    this._connections.forEach((conn) => conn.end());
    this._extDisposables.forEach((disposable) => disposable.dispose());
  }
}

export default SearchQuerySubscription;
