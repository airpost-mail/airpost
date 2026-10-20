// The desktop half of the `query` parity suite (docs/desktop-client-plan.md stage 7).
//
// Two checked-in files under crates/ms-store are generated here and compared with what the
// models produce now, so a change on either side fails one of the two suites:
// - src/model_query_allowlist.json: every `queryable` attribute (and every joined-data
//   attribute) of the classes the engine's `query` serves, with the natural sort orders. The
//   engine compiles only what this table names.
// - tests/fixtures/model_queries.json: the queries the desktop's call sites build, each as
//   `Query.toJSON()` and `Query.sql()`. `crates/ms-store/tests/model_query_parity.rs` runs both
//   forms against one fixture database and compares the rows; the ids below are that
//   database's.
//
// After changing a model or a call site, regenerate both files with
//   TERN_WRITE_QUERY_FIXTURES=1 npm test -- --spec-file-pattern=model-query-capture
// and run `cargo test -p ms-store --test model_query_parity`.
import fs from 'fs';
import path from 'path';
import DatabaseStore from '../src/flux/stores/database-store';
import ModelQuery from '../src/flux/models/query';
import { Attribute } from '../src/flux/attributes/attribute';
import {
  AttributeBoolean,
  AttributeCollection,
  AttributeDateTime,
  AttributeJoinedData,
  AttributeNumber,
  AttributeObject,
  AttributeString,
  Matcher,
} from '../src/flux/attributes';
import { Model } from '../src/flux/models/model';
import { Thread } from '../src/flux/models/thread';
import { Message } from '../src/flux/models/message';
import { Folder } from '../src/flux/models/folder';
import { Label } from '../src/flux/models/label';
import { Contact } from '../src/flux/models/contact';
import { ContactGroup } from '../src/flux/models/contact-group';
import { ContactBook } from '../src/flux/models/contact-book';
import { Event } from '../src/flux/models/event';
import { Calendar } from '../src/flux/models/calendar';
import { File } from '../src/flux/models/file';
import { Task } from '../src/flux/tasks/task';

const REPO = path.resolve(__dirname, '..', '..', '..', '..');
const ALLOWLIST = path.join(REPO, 'crates', 'ms-store', 'src', 'model_query_allowlist.json');
const QUERIES = path.join(REPO, 'crates', 'ms-store', 'tests', 'fixtures', 'model_queries.json');
const WRITE = !!process.env.TERN_WRITE_QUERY_FIXTURES;

/** The classes `query` serves (docs/desktop-client-plan.md 3.2). */
const CLASSES: (typeof Model)[] = [
  Thread,
  Message,
  Folder,
  Label,
  Contact,
  ContactGroup,
  ContactBook,
  Event,
  Calendar,
  Task,
  File,
];

function kindOf(attr: Attribute): string {
  if (attr instanceof AttributeJoinedData) return 'joinedData';
  if (attr instanceof AttributeCollection) return 'collection';
  if (attr instanceof AttributeBoolean) return 'boolean';
  if (attr instanceof AttributeDateTime) return 'datetime';
  if (attr instanceof AttributeNumber) return 'number';
  if (attr instanceof AttributeString) return 'string';
  if (attr instanceof AttributeObject) return 'object';
  throw new Error(`no kind for attribute ${attr.modelKey}`);
}

function allowlist() {
  const classes = {};
  for (const klass of CLASSES) {
    const attributes = {};
    for (const key of Object.keys(klass.attributes).sort()) {
      const attr = klass.attributes[key];
      const joined = attr instanceof AttributeJoinedData;
      if (!attr.queryable && !joined) continue;
      const kind = kindOf(attr);
      const entry: Record<string, unknown> = { kind };
      if (attr instanceof AttributeCollection) {
        entry.joinTable = attr.tableNameForJoinAgainst(klass);
        entry.joinQueryableBy = [...attr.joinQueryableBy];
      } else if (attr instanceof AttributeJoinedData) {
        entry.table = attr.modelTable;
      } else {
        entry.column = attr.tableColumn;
      }
      attributes[attr.modelKey] = entry;
    }
    const natural = klass.naturalSortOrder();
    classes[klass.name] = {
      attributes,
      naturalOrder: natural
        ? { attr: natural.attr.modelKey, dir: natural.direction === 'ASC' ? 'asc' : 'desc' }
        : null,
    };
  }
  return {
    generatedBy: 'apps/desktop/app/spec/model-query-capture-spec.ts',
    classes,
  };
}

// Ids of the fixture database in crates/ms-store/tests/model_query_parity.rs.
const BASE = 1_700_000_000;
const at = (offset: number) => new Date((BASE + offset) * 1000);
const A1 = 'a1';
const A2 = 'a2';

type Site = { name: string; site: string; build: () => ModelQuery<any> };

/** Builds a query the way `site` does; `range` mirrors QuerySubscription's paging. */
const SITES: Site[] = [
  // Thread lists: MailboxPerspective.threads(), paged by QuerySubscription._getQueryForRange.
  {
    name: 'inbox-page-1',
    site: 'src/mailbox-perspective.ts CategoryMailboxPerspective.threads',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([Thread.attributes.categories.containsAny(['f-a1-inbox'])])
        .where({ inAllMail: true })
        .offset(0)
        .limit(10),
  },
  {
    name: 'inbox-page-2',
    site: 'src/mailbox-perspective.ts CategoryMailboxPerspective.threads',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([Thread.attributes.categories.containsAny(['f-a1-inbox'])])
        .where({ inAllMail: true })
        .offset(10)
        .limit(10),
  },
  {
    name: 'inbox-ids',
    site: 'src/flux/models/query-subscription.ts _getQueryForRange (idsOnly)',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([Thread.attributes.categories.containsAny(['f-a1-inbox'])])
        .where({ inAllMail: true })
        .offset(0)
        .limit(100)
        .idsOnly(),
  },
  {
    name: 'unified-inbox',
    site: 'src/mailbox-perspective.ts CategoryMailboxPerspective.threads',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([Thread.attributes.categories.containsAny(['f-a1-inbox', 'f-a2-inbox'])])
        .where({ inAllMail: true })
        .offset(0)
        .limit(100),
  },
  {
    name: 'sent',
    site: 'src/mailbox-perspective.ts CategoryMailboxPerspective.threads (isSent)',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([Thread.attributes.categories.containsAny(['f-a1-sent', 'f-a2-sent'])])
        .order(Thread.attributes.lastMessageSentTimestamp.descending())
        .where({ inAllMail: true })
        .offset(0)
        .limit(100),
  },
  {
    name: 'trash',
    site: 'src/mailbox-perspective.ts CategoryMailboxPerspective.threads (spam, trash)',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([Thread.attributes.categories.containsAny(['f-a1-trash'])])
        .offset(0)
        .limit(100),
  },
  {
    name: 'two-categories-distinct',
    site: 'src/mailbox-perspective.ts CategoryMailboxPerspective.threads (distinct)',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([Thread.attributes.categories.containsAny(['f-a1-inbox', 'f-a1-work'])])
        .where({ inAllMail: true })
        .distinct()
        .offset(0)
        .limit(100),
  },
  {
    name: 'label',
    site: 'src/mailbox-perspective.ts CategoryMailboxPerspective.threads (label)',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([Thread.attributes.categories.containsAny(['l-a2-travel'])])
        .where({ inAllMail: true })
        .offset(0)
        .limit(100),
  },
  {
    name: 'starred-all-accounts',
    site: 'src/mailbox-perspective.ts StarredMailboxPerspective.threads',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([Thread.attributes.starred.equal(true), Thread.attributes.inAllMail.equal(true)])
        .offset(0)
        .limit(100),
  },
  {
    name: 'starred-one-account',
    site: 'src/mailbox-perspective.ts StarredMailboxPerspective.threads (accountId in)',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([Thread.attributes.starred.equal(true), Thread.attributes.inAllMail.equal(true)])
        .where(Thread.attributes.accountId.in([A2]))
        .offset(0)
        .limit(100),
  },
  {
    name: 'empty-perspective',
    site: 'src/mailbox-perspective.ts EmptyMailboxPerspective.threads',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread).where({ lastMessageReceivedTimestamp: -1 }).limit(0),
  },
  {
    name: 'unread',
    site: 'src/flux/models/unread-query-subscription.ts buildQuery',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([
          Thread.attributes.categories.containsAny(['f-a1-inbox']),
          Thread.attributes.unread.equal(true),
        ])
        .offset(0)
        .limit(100),
  },
  {
    name: 'unread-recently-read',
    site: 'src/flux/models/unread-query-subscription.ts buildQuery (recentlyReadIds)',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where([
          Thread.attributes.categories.containsAny(['f-a1-inbox']),
          new Matcher.Or([
            Thread.attributes.unread.equal(true),
            Thread.attributes.id.in(['t02', 't04']),
          ]),
        ])
        .offset(0)
        .limit(100),
  },
  {
    name: 'mark-all-read',
    site: 'features/account-sidebar/lib/sidebar-item.ts onMarkAllAsRead',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread).where([
        Thread.attributes.categories.containsAny(['f-a1-inbox']),
        Thread.attributes.unread.equal(true),
        Thread.attributes.inAllMail.equal(true),
      ]),
  },
  {
    name: 'permalink-date',
    site: 'features/thread-list/lib/thread-permalink-handler.ts (date)',
    build: () =>
      DatabaseStore.findBy<Thread>(Thread).where([
        Thread.attributes.subject.equal('Subject 07'),
        new Matcher.And([
          Thread.attributes.firstMessageTimestamp.lessThan(at(7000 + 120)),
          Thread.attributes.firstMessageTimestamp.greaterThan(at(7000 - 120)),
        ]),
      ]),
  },
  {
    name: 'permalink-last-date',
    site: 'features/thread-list/lib/thread-permalink-handler.ts (lastDate)',
    build: () =>
      DatabaseStore.findBy<Thread>(Thread).where([
        Thread.attributes.subject.equal('Subject 08'),
        new Matcher.Or([
          new Matcher.And([
            Thread.attributes.lastMessageSentTimestamp.lessThan(at(8500 + 120)),
            Thread.attributes.lastMessageSentTimestamp.greaterThan(at(8500 - 120)),
          ]),
          new Matcher.And([
            Thread.attributes.lastMessageReceivedTimestamp.lessThan(at(8500 + 120)),
            Thread.attributes.lastMessageReceivedTimestamp.greaterThan(at(8500 - 120)),
          ]),
        ]),
      ]),
  },
  {
    name: 'mail-rules-inbox',
    site: 'src/flux/stores/mail-rules-store.ts _processAllMessages',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread, { accountId: A1 })
        .where(Thread.attributes.categories.contains('f-a1-inbox'))
        .order(Thread.attributes.lastMessageReceivedTimestamp.descending())
        .limit(5),
  },
  {
    name: 'mail-rules-inbox-before',
    site: 'src/flux/stores/mail-rules-store.ts _processAllMessages (lastTimestamp)',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread, { accountId: A1 })
        .where(Thread.attributes.categories.contains('f-a1-inbox'))
        .order(Thread.attributes.lastMessageReceivedTimestamp.descending())
        .limit(5)
        .where(Thread.attributes.lastMessageReceivedTimestamp.lessThan(at(12000))),
  },
  {
    name: 'search-result-ids',
    site: 'features/thread-search/lib/search-query-subscription.ts _onSearchResults',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread)
        .where({ id: ['t01', 't05', 't30', 'missing'] })
        .order(Thread.attributes.lastMessageReceivedTimestamp.descending()),
  },
  {
    name: 'thread-by-id',
    site: 'DatabaseStore.find(Thread) (draft-store, message-list, activity, mail rules)',
    build: () => DatabaseStore.find<Thread>(Thread, 't03'),
  },
  {
    name: 'thread-findby-id-limit',
    site: 'features/thread-list/lib/thread-list-context-menu.ts copyPermalink',
    build: () => DatabaseStore.findBy<Thread>(Thread, { id: 't04' }).limit(1),
  },
  {
    name: 'threads-by-ids',
    site: 'features/unread-notifications/lib/main.ts _threadsForMessages',
    build: () => DatabaseStore.findAll<Thread>(Thread, Thread.attributes.id.in(['t02', 't26'])),
  },
  {
    name: 'missing-models',
    site: 'src/flux/models/query-subscription.ts _fetchMissingModels',
    build: () => DatabaseStore.findAll<Thread>(Thread, { id: ['t09', 't10', 't11'] }),
  },
  {
    name: 'thread-count',
    site: 'DatabaseStore.count',
    build: () =>
      DatabaseStore.count<Thread>(Thread, [
        Thread.attributes.categories.containsAny(['f-a1-inbox']),
        Thread.attributes.unread.equal(true),
      ]),
  },
  {
    name: 'snoozed-threads',
    site: 'ModelWithMetadata.pluginMetadata on Thread',
    build: () =>
      DatabaseStore.findAll<Thread>(Thread).where(
        Thread.attributes.pluginMetadata.contains('thread-snooze')
      ),
  },

  // Messages.
  {
    name: 'thread-messages',
    site: 'src/flux/stores/message-store.ts _fetchFromCache',
    build: () => {
      const q = DatabaseStore.findAll<Message>(Message);
      q.where({ threadId: 't03' });
      q.include(Message.attributes.body);
      return q;
    },
  },
  {
    name: 'thread-messages-no-body',
    site: 'features/thread-list/lib/thread-list-data-source.ts _observableForThreadMessages',
    build: () => DatabaseStore.findAll<Message>(Message, { threadId: 't05' }),
  },
  {
    name: 'messages-of-threads',
    site: 'features/thread-list/lib/thread-list-data-source.ts (missingIds)',
    build: () => DatabaseStore.findAll<Message>(Message, { threadId: ['t01', 't02', 't27'] }),
  },
  {
    name: 'latest-not-draft',
    site: 'src/flux/stores/draft-factory.ts createDraftForResurfacing',
    build: () =>
      DatabaseStore.findBy<Message>(Message, { threadId: 't06' })
        .where(Message.attributes.draft.equal(false))
        .order(Message.attributes.date.descending())
        .limit(1),
  },
  {
    name: 'reply-candidates',
    site: 'src/flux/stores/draft-store.ts _getMessageAndThread',
    build: () =>
      DatabaseStore.findAll<Message>(Message, { threadId: 't06' })
        .where(Message.attributes.draft.equal(false))
        .order(Message.attributes.date.descending()),
  },
  {
    name: 'eml-latest',
    site: 'src/services/eml-utils.ts latestMessages',
    build: () =>
      DatabaseStore.findAll<Message>(Message, { threadId: 't07', draft: false })
        .order(Message.attributes.date.descending())
        .limit(1),
  },
  {
    name: 'message-by-id-with-body',
    site: 'src/flux/stores/draft-store.ts _getMessageAndThread',
    build: () => DatabaseStore.find<Message>(Message, 'm03-2').include(Message.attributes.body),
  },
  {
    name: 'message-pending-body',
    site: 'src/flux/stores/draft-store.ts (body not fetched yet)',
    build: () => DatabaseStore.find<Message>(Message, 'm04-1').include(Message.attributes.body),
  },
  {
    name: 'message-by-id',
    site: 'DatabaseStore.find(Message) (event-rsvp-task, undo-redo-toast)',
    build: () => DatabaseStore.find<Message>(Message, 'm10-1'),
  },
  {
    name: 'draft-by-header',
    site: 'src/flux/stores/draft-editing-session.ts prepare',
    build: () =>
      DatabaseStore.findBy<Message>(Message, {
        headerMessageId: '<d02@x.test>',
        draft: true,
      }).include(Message.attributes.body),
  },
  {
    name: 'draft-list',
    site: 'features/draft-list/lib/draft-list-store.ts _onPerspectiveChanged',
    build: () =>
      DatabaseStore.findAll<Message>(Message)
        .include(Message.attributes.body)
        .order(Message.attributes.date.descending())
        .where({ draft: true })
        .page(0, 1),
  },
  {
    name: 'draft-list-one-account',
    site: 'features/draft-list/lib/draft-list-store.ts (accountId)',
    build: () =>
      DatabaseStore.findAll<Message>(Message)
        .include(Message.attributes.body)
        .order(Message.attributes.date.descending())
        .where({ draft: true })
        .page(0, 1)
        .where({ accountId: [A1] }),
  },
  {
    name: 'messages-by-ids',
    site: 'features/unread-notifications/lib/main.ts _onNewMessages',
    build: () =>
      DatabaseStore.findAll<Message>(Message, Message.attributes.id.in(['m01-1', 'm25-2'])),
  },
  {
    name: 'activity',
    site: 'plugins/activity/lib/activity-data-source.ts buildQuery',
    build: () =>
      DatabaseStore.findAll<Message>(Message)
        .order(Message.attributes.date.descending())
        .where(Message.attributes.pluginMetadata.containsAny(['open-tracking', 'link-tracking']))
        .where(Message.attributes.draft.equal(false))
        .distinct()
        .limit(20),
  },
  {
    name: 'activity-accounts-dates',
    site: 'plugins/activity/lib/activity-data-source.ts buildQuery (filters)',
    build: () =>
      DatabaseStore.findAll<Message>(Message)
        .order(Message.attributes.date.descending())
        .where(Message.attributes.pluginMetadata.containsAny(['open-tracking', 'link-tracking']))
        .where(Message.attributes.draft.equal(false))
        .distinct()
        .where(Message.attributes.accountId.in([A1, A2]))
        .where(Message.attributes.date.greaterThan(at(3000)))
        .where(Message.attributes.date.lessThan(at(30000))),
  },
  {
    name: 'message-scan',
    site: 'plugins/activity/lib/message-scan.ts',
    build: () =>
      DatabaseStore.findAll<Message>(Message)
        .background()
        .where(Message.attributes.accountId.in([A1, A2]))
        .where(Message.attributes.date.greaterThan(at(5000)))
        .where(Message.attributes.date.lessThan(at(25000)))
        .order(Message.attributes.date.ascending())
        .limit(15),
  },

  // Categories and tasks.
  {
    name: 'folders',
    site: 'src/global/mailspring-observables.ts categories',
    build: () => DatabaseStore.findAll<Folder>(Folder),
  },
  {
    name: 'labels-for-account',
    site: 'src/global/mailspring-observables.ts forAccount',
    build: () => DatabaseStore.findAll<Label>(Label).where({ accountId: A2 }),
  },
  {
    name: 'tasks',
    site: 'src/flux/stores/task-queue.ts',
    build: () => DatabaseStore.findAll<Task>(Task),
  },

  // Contacts.
  {
    name: 'contact-search',
    site: 'src/flux/stores/contact-store.ts searchContacts',
    build: () =>
      DatabaseStore.findAll<Contact>(Contact)
        .search('ann')
        .limit(10 * 2)
        .where(Contact.attributes.refs.greaterThan(0))
        .where(Contact.attributes.hidden.equal(false))
        .order(Contact.attributes.refs.descending()),
  },
  {
    name: 'contact-search-quoted',
    site: 'src/flux/stores/contact-store.ts searchContacts (quotes)',
    build: () =>
      DatabaseStore.findAll<Contact>(Contact)
        .search(" 'bo")
        .limit(20)
        .where(Contact.attributes.refs.greaterThan(0))
        .where(Contact.attributes.hidden.equal(false))
        .order(Contact.attributes.refs.descending()),
  },
  {
    name: 'top-contacts',
    site: 'src/flux/stores/contact-store.ts topContacts',
    build: () =>
      DatabaseStore.findAll<Contact>(Contact)
        .limit(5 * 2)
        .where(Contact.attributes.refs.greaterThan(0))
        .where(Contact.attributes.hidden.equal(false))
        .order(Contact.attributes.refs.descending()),
  },
  {
    name: 'contacts-list',
    site: 'features/contacts/lib/Store.ts',
    build: () =>
      DatabaseStore.findAll<Contact>(Contact)
        .where(Contact.attributes.refs.greaterThan(0))
        .where(Contact.attributes.hidden.equal(false)),
  },
  {
    name: 'contacts-not-mail',
    site: 'features/contacts/lib/Store.ts (source)',
    build: () =>
      DatabaseStore.findAll<Contact>(Contact)
        .where(Contact.attributes.refs.greaterThan(0))
        .where(Contact.attributes.hidden.equal(false))
        .where(Contact.attributes.source.not('mail')),
  },
  {
    name: 'contacts-in-groups',
    site: 'src/components/participants-text-field.tsx _onAdd',
    build: () =>
      DatabaseStore.findAll<Contact>(Contact, [
        Contact.attributes.contactGroups.containsAny(['g1', 'g2']),
      ]),
  },
  {
    name: 'focused-contact',
    site: 'src/flux/stores/focused-contacts-store.ts',
    build: () => DatabaseStore.findBy<Contact>(Contact, { accountId: A1, email: 'c03@x.test' }),
  },
  {
    name: 'contact-groups',
    site: 'src/flux/stores/contact-store.ts searchContactGroups',
    build: () => DatabaseStore.findAll<ContactGroup>(ContactGroup),
  },
  {
    name: 'contact-books',
    site: 'features/contacts/lib/Store.ts',
    build: () => DatabaseStore.findAll<ContactBook>(ContactBook),
  },

  // Calendar.
  {
    name: 'calendars',
    site: 'features/main-calendar/lib/core/calendar-sidebar.tsx',
    build: () => DatabaseStore.findAll<Calendar>(Calendar),
  },
  {
    name: 'calendars-for-account',
    site: 'features/events/lib/event-header.tsx',
    build: () => DatabaseStore.findAll<Calendar>(Calendar).where({ accountId: A1 }),
  },
  {
    name: 'calendar-by-id',
    site: 'features/main-calendar/lib/core/calendar-rsvp.ts',
    build: () => DatabaseStore.find<Calendar>(Calendar, 'cal2'),
  },
  {
    name: 'events-in-range',
    site: 'features/main-calendar/lib/core/calendar-data-source.ts buildObservable',
    build: () => {
      const end = Event.attributes.recurrenceEnd;
      const start = Event.attributes.recurrenceStart;
      const [startUnix, endUnix] = [BASE + 20000, BASE + 60000];
      const dateMatcher = new Matcher.Or([
        new Matcher.And([start.lte(endUnix), end.gte(startUnix)]),
        new Matcher.And([start.lte(endUnix), start.gte(startUnix)]),
        new Matcher.And([end.gte(startUnix), end.lte(endUnix)]),
        new Matcher.And([end.gte(endUnix), start.lte(startUnix)]),
      ]);
      return DatabaseStore.findAll<Event>(Event).where(
        new Matcher.And([dateMatcher, Event.attributes.calendarId.notIn(['cal2', 'nope'])])
      );
    },
  },
  {
    name: 'events-by-uid',
    site: 'features/main-calendar/lib/main.tsx, events/lib/event-header.tsx',
    build: () => DatabaseStore.findAll<Event>(Event).where({ icsuid: 'uid-03', accountId: A1 }),
  },
  {
    name: 'events-nearby',
    site: 'features/events/lib/event-header.tsx (conflicts)',
    build: () =>
      DatabaseStore.findAll<Event>(Event).where([
        Event.attributes.accountId.equal(A1),
        Event.attributes.recurrenceStart.lessThan(BASE + 40000),
        Event.attributes.recurrenceEnd.greaterThan(BASE + 10000),
      ]),
  },
  {
    name: 'event-master',
    site: 'features/main-calendar/lib/core/calendar-view.tsx _onDelete',
    build: () =>
      DatabaseStore.findBy<Event>(Event, {
        accountId: A1,
        calendarId: 'cal1',
        icsuid: 'uid-03',
        recurrenceId: '',
      }),
  },
  {
    name: 'event-by-id',
    site: 'DatabaseStore.find(Event) (calendar-rsvp, popovers, calendar-view)',
    build: () => DatabaseStore.find<Event>(Event, 'e05'),
  },
  {
    name: 'event-search',
    site: 'features/main-calendar/lib/core/event-search-bar.tsx',
    build: () =>
      DatabaseStore.findAll<Event>(Event)
        .distinct()
        .where(Event.attributes.calendarId.notIn(['cal3', 'x']))
        .search('lunch')
        .limit(10),
  },
  {
    name: 'file-by-id',
    site: 'DatabaseStore.find(File)',
    build: () => DatabaseStore.find<File>(File, 'file2'),
  },
];

function capture() {
  // Join aliases come from a global counter; reset it so the SQL doesn't depend on which specs
  // ran first.
  Matcher.muid = 0;
  return {
    generatedBy: 'apps/desktop/app/spec/model-query-capture-spec.ts',
    queries: SITES.map(({ name, site, build }) => {
      const json = build().toJSON();
      const sql = build().sql();
      return { name, site, query: json, sql };
    }),
  };
}

const serialize = (value: unknown) => `${JSON.stringify(value, null, 2)}\n`;

function compareWithFile(file: string, value: unknown) {
  const text = serialize(value);
  if (WRITE) fs.writeFileSync(file, text);
  const onDisk = fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : '';
  if (onDisk !== text) {
    throw new Error(
      `${path.relative(REPO, file)} is out of date; regenerate it with TERN_WRITE_QUERY_FIXTURES=1`
    );
  }
}

describe('ModelQuery capture for the engine', () => {
  it('matches the engine allowlist', () => {
    compareWithFile(ALLOWLIST, allowlist());
  });

  it('matches the captured call-site queries', () => {
    compareWithFile(QUERIES, capture());
  });

  it('sends booleans and dates as the SQL compares them', () => {
    const json = DatabaseStore.findAll<Thread>(Thread)
      .where(Thread.attributes.unread.equal(true))
      .where(Thread.attributes.lastMessageReceivedTimestamp.lessThan(at(10)))
      .toJSON();
    expect(json.where).toEqual({
      and: [
        { attr: 'unread', op: '!=', value: 0 },
        { attr: 'lastMessageReceivedTimestamp', op: '<', value: BASE + 10 },
      ],
    });
    expect(json.order).toEqual([{ attr: 'lastMessageReceivedTimestamp', dir: 'desc' }]);
  });

  it('sends a plain search as search text and refuses a structured one', () => {
    const json = DatabaseStore.findAll<Contact>(Contact).search(' "ann ').toJSON();
    expect(json.search).toEqual({ text: ' "ann ', grammar: false });
    expect(json.where).toBe(null);
    const structured = DatabaseStore.findAll<Thread>(Thread).structuredSearch(null);
    expect(() => structured.toJSON()).toThrow();
  });
});
