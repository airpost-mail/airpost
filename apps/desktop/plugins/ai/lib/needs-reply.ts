// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/*
"Needs reply" in the sidebar, for each account with triage on: the inbox threads the engine
triaged needs-reply (`ai.triage.list`), newest first. The list is asked again whenever threads
change, since a triage result or a correction arrives as a Thread delta.
*/
import _ from 'underscore';
import {
  DatabaseStore,
  MailboxPerspective,
  MutableQuerySubscription,
  Thread,
  localized,
} from 'mailspring-exports';
import { listTriaged, triageShownFor } from './triage';

const RELIST_DEBOUNCE_MS = 500;

function threadsWithIds(ids: string[]) {
  // No thread has the empty id: an empty list matches nothing without an empty IN ().
  const matcher = ids.length ? Thread.attributes.id.in(ids) : Thread.attributes.id.equal('');
  return DatabaseStore.findAll<Thread>(Thread)
    .where(matcher)
    .order(Thread.attributes.lastMessageReceivedTimestamp.descending());
}

export class NeedsReplySubscription extends MutableQuerySubscription<Thread> {
  accountIds: string[];
  _unlisten: () => void;
  _listing = 0;

  constructor(accountIds: string[]) {
    super(threadsWithIds([]), { emitResultSet: true });
    this.accountIds = accountIds;
    this._unlisten = DatabaseStore.listen(this._onChange);
    this.relist();
  }

  _onChange = (change: { objectClass?: string }) => {
    if (change && change.objectClass === Thread.name) this._relistSoon();
  };

  _relistSoon = _.debounce(() => this.relist(), RELIST_DEBOUNCE_MS);

  async relist() {
    const call = ++this._listing;
    try {
      const { threadIds } = await listTriaged(this.accountIds, 'needs-reply');
      if (call !== this._listing) return;
      const next = threadsWithIds(threadIds);
      const range = this._query && this._query.range();
      if (range && !range.isInfinite()) next.offset(range.offset).limit(range.limit);
      this.replaceQuery(next);
    } catch (err) {
      // The engine is down or triage was turned off: keep what is shown.
    }
  }

  onLastCallbackRemoved() {
    this._unlisten();
    this._relistSoon.cancel();
  }
}

export class NeedsReplyMailboxPerspective extends MailboxPerspective {
  name = localized('Needs reply');
  iconName = 'today.png';

  emptyMessage() {
    return localized('Nothing needs a reply');
  }

  threads() {
    return new NeedsReplySubscription(this.accountIds);
  }

  // Nothing is dropped into the view; its threads move, archive and trash as inbox threads do.
  canReceiveThreadsFromAccountIds() {
    return false;
  }
}

export const NeedsReplySidebarExtension = {
  name: 'AiNeedsReplySidebarExtension',

  /** The item for the accounts of `accountIds` that get triage; null, and no item, for none. */
  sidebarItem(accountIds: string[]) {
    const triaged = accountIds.filter((id) => triageShownFor(id));
    if (!triaged.length) return null;
    return {
      id: 'AiNeedsReply',
      name: localized('Needs reply'),
      iconName: 'today.png',
      perspective: new NeedsReplyMailboxPerspective(triaged),
      insertAtTop: true,
    };
  },
};
