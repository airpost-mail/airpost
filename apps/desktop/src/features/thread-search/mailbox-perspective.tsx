import React from 'react';
import { localized } from '../../intl/localize';
import { AccountStore } from '../../data/stores/account';
import { TaskFactory } from '../../data/tasks/factory';
import { MailboxPerspective } from '../../data/mailbox-perspective';
import { Thread } from '../../data/models/thread';
import * as Actions from '../../data/actions';
import SearchQuerySubscription from './query-subscription';
import { Button } from '../../components/kit/button';

class SearchMailboxPerspective extends MailboxPerspective {
  searchQuery: string;
  sourcePerspective: MailboxPerspective;
  name: string;

  constructor(sourcePerspective: MailboxPerspective, searchQuery: string) {
    super(sourcePerspective.accountIds);
    if (typeof searchQuery !== 'string') {
      throw new Error('SearchMailboxPerspective: Expected a `string` search query');
    }

    this.searchQuery = searchQuery;

    if (sourcePerspective instanceof SearchMailboxPerspective) {
      this.sourcePerspective = sourcePerspective.sourcePerspective;
    } else {
      this.sourcePerspective = sourcePerspective;
    }

    this.name = `Searching ${this.sourcePerspective.name}`;
  }

  emptyMessage() {
    const inTrash = this.isSearchingTrash();

    return (
      <span>
        {localized('No search results')}
        {!inTrash && (
          <div>
            <Button
              variant="outline"
              style={{ fontWeight: 'normal' }}
              onClick={() =>
                Actions.searchQuerySubmitted(`${this.searchQuery} (in:trash OR in:spam)`)
              }
            >
              {localized('Search messages in trash and spam')}
            </Button>
          </div>
        )}
      </span>
    );
  }

  isSearchingTrash() {
    return /in: ?['"]?(trash|spam)/gi.test(this.searchQuery);
  }

  isEqual(other: MailboxPerspective) {
    // super.isEqual checks that the constructors match.
    return (
      super.isEqual(other) &&
      other instanceof SearchMailboxPerspective &&
      other.searchQuery === this.searchQuery
    );
  }

  threads() {
    // If your query doesn't explicitly ask for results in trash or in spam, we exclude
    // them to increase the quality of results, and a button in the empty state (above)
    // allows you to switch to showing trash results.
    let finalQuery = this.searchQuery.trim();
    if (!this.isSearchingTrash()) {
      finalQuery = `(${finalQuery}) NOT (in:trash OR in:spam)`;
    }

    return new SearchQuerySubscription(finalQuery, this.accountIds);
  }

  canReceiveThreadsFromAccountIds() {
    return false;
  }

  // Search results span every folder, so "remove" takes each account's usual archive or
  // trash action without scoping it to a folder: the engine moves every copy outside Sent
  // and Drafts (on Gmail, archiving removes the Inbox label).
  _removalOf(thread: Thread) {
    const dest = AccountStore.accountForId(thread.accountId)?.preferredRemovalDestination();
    return dest ? (dest.role === 'trash' ? ('trash' as const) : ('archive' as const)) : null;
  }

  removalKind(threads: Thread[]) {
    return threads.length ? this._removalOf(threads[0]) : null;
  }

  tasksForRemovingItems(threads: Thread[], source = 'Dragged out of list') {
    const trash = threads.filter((t) => this._removalOf(t) === 'trash');
    const archive = threads.filter((t) => this._removalOf(t) === 'archive');
    return [
      ...TaskFactory.tasksForMovingToTrash({ threads: trash, source }),
      ...TaskFactory.tasksForArchiving({ threads: archive, source }),
    ];
  }
}

export default SearchMailboxPerspective;
