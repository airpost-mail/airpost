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

  // Search results span every folder, so "remove" takes the account's usual archive or
  // trash action without scoping it to a folder: the engine moves every copy outside Sent
  // and Drafts. On Gmail the archive destination is the All Mail folder, where the message
  // already is, so archiving has to go through TaskFactory's label removal instead.
  tasksForRemovingItems(threads: Thread[], source = 'Dragged out of list') {
    return TaskFactory.tasksForThreadsByAccountId(threads, (accountThreads, accountId) => {
      const dest = AccountStore.accountForId(accountId)?.preferredRemovalDestination();
      if (!dest) {
        return [];
      }
      if (dest.role === 'trash') {
        return TaskFactory.tasksForMovingToTrash({ threads: accountThreads, source });
      }
      return TaskFactory.tasksForArchiving({ threads: accountThreads, source });
    });
  }
}

export default SearchMailboxPerspective;
