import React from 'react';
import ReactDOM from 'react-dom';
import ReactTestUtils from 'react-dom/test-utils';
import fs from 'fs';
import moment from 'moment';
import os from 'os';
import path from 'path';
import { Account } from '../../src/data/models/account';
import { AccountStore } from '../../src/data/stores/account';
import * as Actions from '../../src/data/actions';
import AttachmentStore from '../../src/data/stores/attachment';
import { Calendar } from '../../src/data/models/calendar';
import { Contact } from '../../src/data/models/contact';
import { Event } from '../../src/data/models/event';
import { File } from '../../src/data/models/file';
import { Message } from '../../src/data/models/message';
import Rx from 'rx-lite';
import { SyncbackEventTask } from '../../src/data/tasks/syncback-event';
import { Task } from '../../src/data/tasks/task';
import * as Utils from '../../src/data/models/utils';
import { EventHeader, renderLocation } from '../../src/features/events/header';
import type { EventConflictJSON, EventTimes } from '../../src/engine/protocol';
import { fakePimEngine, refusal } from '../support/fake-pim-engine';
import { getPortalContainer } from '../../src/components/kit/portal';

// A LOCATION as Zoom rooms write it: the join URL, then the rooms booked for the meeting.
const ZOOM_WITH_ROOMS =
  'https://sequoia.zoom.us/j/7647885554, London-2-Oxford (7) [Zoom Room], MP2800-2-DuPont (7) [Zoom Room]';

type Link = React.ReactElement<{ href: string; children: string }>;

function linksIn(node: React.ReactNode): Array<{ href: string; text: string }> {
  return (Array.isArray(node) ? node : [node])
    .filter((n): n is Link => React.isValidElement(n))
    .map((n) => ({ href: n.props.href, text: n.props.children }));
}

function textOf(node: React.ReactNode): string {
  return (Array.isArray(node) ? node : [node])
    .map((n) => (React.isValidElement(n) ? (n as Link).props.children : n))
    .join('');
}

describe('EventHeader location', function () {
  it('links a join URL to itself', function () {
    const node = renderLocation('https://meet.google.com/abc-defg-hij');
    expect(linksIn(node)).toEqual([
      {
        href: 'https://meet.google.com/abc-defg-hij',
        text: 'https://meet.google.com/abc-defg-hij',
      },
    ]);
  });

  it('links only the URL when rooms follow it, and keeps the rooms as text', function () {
    const node = renderLocation(ZOOM_WITH_ROOMS);
    expect(linksIn(node)).toEqual([
      {
        href: 'https://sequoia.zoom.us/j/7647885554',
        text: 'https://sequoia.zoom.us/j/7647885554',
      },
    ]);
    expect(textOf(node)).toBe(ZOOM_WITH_ROOMS);
  });

  it('links a URL that comes after text', function () {
    const node = renderLocation('Dial in: see https://example.com/notes');
    expect(linksIn(node)).toEqual([
      { href: 'https://example.com/notes', text: 'https://example.com/notes' },
    ]);
    expect(textOf(node)).toBe('Dial in: see https://example.com/notes');
  });

  it('links a tel: URI the way the event card does', function () {
    const node = renderLocation('tel:+1-415-555-1234,,12345#, Boardroom');
    expect(linksIn(node)).toEqual([
      { href: 'tel:+1-415-555-1234,,12345#', text: 'tel:+1-415-555-1234,,12345#' },
    ]);
    expect(textOf(node)).toBe('tel:+1-415-555-1234,,12345#, Boardroom');
  });

  it('links a URL with tel: in its path once, as the URL', function () {
    const node = renderLocation('https://example.com/dial/tel:+15550100');
    expect(linksIn(node)).toEqual([
      {
        href: 'https://example.com/dial/tel:+15550100',
        text: 'https://example.com/dial/tel:+15550100',
      },
    ]);
  });

  it('leaves a room name as text', function () {
    expect(renderLocation('Conference Room 4B')).toBe('Conference Room 4B');
    expect(renderLocation('MP2800-2-DuPont (7) [Zoom Room]')).toBe(
      'MP2800-2-DuPont (7) [Zoom Room]'
    );
  });

  it('renders nothing for a missing location', function () {
    expect(renderLocation(undefined)).toBeNull();
    expect(renderLocation('')).toBeNull();
  });

  describe('rendered from an invitation attachment', function () {
    let icsPath: string;

    beforeEach(function () {
      icsPath = path.join(os.tmpdir(), `event-header-spec-${process.pid}.ics`);
      fs.writeFileSync(
        icsPath,
        [
          'BEGIN:VCALENDAR',
          'VERSION:2.0',
          'PRODID:-//Test//Test//EN',
          'METHOD:REQUEST',
          'BEGIN:VEVENT',
          'UID:zoom-rooms@test',
          'DTSTART:20260924T150000Z',
          'DTEND:20260924T153000Z',
          'SUMMARY:Planning',
          `LOCATION:${ZOOM_WITH_ROOMS.replace(/,/g, '\\,')}`,
          'DTSTAMP:20260901T000000Z',
          'END:VEVENT',
          'END:VCALENDAR',
        ].join('\r\n')
      );
      spyOn(AttachmentStore, 'pathForFile').andReturn(icsPath);
      spyOn(Rx.Observable, 'fromQuery').andReturn(Rx.Observable.empty());
    });

    afterEach(function () {
      fs.unlinkSync(icsPath);
    });

    it('shows the join URL as a link and the rooms as text', function () {
      const header = ReactTestUtils.renderIntoDocument(
        <EventHeader
          message={new Message({ accountId: 'a1' })}
          file={new File({ id: 'f1', filename: 'invite.ics' })}
        />
      ) as unknown as EventHeader;
      let location: HTMLElement;
      waitsFor(() => {
        const nodes = ReactTestUtils.scryRenderedDOMComponentsWithClass(header, 'event-location');
        location = nodes[0] as HTMLElement;
        return !!location;
      });
      runs(() => {
        const links = Array.from(location.querySelectorAll('a'));
        expect(links.map((a) => a.getAttribute('href'))).toEqual([
          'https://sequoia.zoom.us/j/7647885554',
        ]);
        expect(location.textContent).toBe(ZOOM_WITH_ROOMS);
      });
    });
  });
});

// The engine reads the invitation's times (events.expand: `first`, `upcoming`), finds the
// conflicts and answers (events.rsvp); this stands in for it. The ICS it builds is crates/tern-pim's
// tests (plan.rs rsvp_*, counter_*).
const unfold = (ics: string) => ics.replace(/\r?\n[ \t]/g, '');

function unixOf(value: string) {
  const m = /^(\d{4})(\d{2})(\d{2})(?:T(\d{2})(\d{2})(\d{2})Z)?$/.exec(value);
  return Date.UTC(+m[1], +m[2] - 1, +m[3], +(m[4] || 0), +(m[5] || 0), +(m[6] || 0)) / 1000;
}

/** What the engine answers as `first`: the times of the file's first VEVENT. */
function firstOf(ics: string): EventTimes {
  const vevent = unfold(ics).split('BEGIN:VEVENT')[1];
  const prop = (name: string) => {
    const m = new RegExp(`^${name}[;:][^\\r\\n]*`, 'm').exec(vevent);
    return m ? m[0].split(':').pop() : undefined;
  };
  const rid = prop('RECURRENCE-ID');
  return {
    start: unixOf(prop('DTSTART')),
    end: unixOf(prop('DTEND')),
    isAllDay: !prop('DTSTART').includes('T'),
    ...(rid ? { recurrenceId: unixOf(rid) } : {}),
  };
}

function pimEngine() {
  const engine = fakePimEngine();
  const answers = {
    upcoming: (_ics: string): EventTimes | null => null,
    conflicts: (): EventConflictJSON[] => [],
  };
  engine.answers['events.expand'] = (params) =>
    params.ics
      ? { occurrences: [], first: firstOf(params.ics), upcoming: answers.upcoming(params.ics) }
      : { occurrences: [], conflicts: params.conflicts ? answers.conflicts() : undefined };
  engine.answers['events.rsvp'] = () => ({ email: 'me' });
  return Object.assign(engine, { expand: answers });
}

describe('EventHeader for an invitation to one occurrence of a series', function () {
  const vcalendar = (...vevents: string[][]) =>
    [
      'BEGIN:VCALENDAR',
      'VERSION:2.0',
      'PRODID:-//Test//Test//EN',
      ...vevents.flatMap((lines) => [
        'BEGIN:VEVENT',
        'UID:huddle@test',
        ...lines,
        'DTSTAMP:20260901T000000Z',
        'END:VEVENT',
      ]),
      'END:VCALENDAR',
    ].join('\r\n');

  const GUESTS = [
    'ORGANIZER:mailto:ada@example.com',
    'ATTENDEE;PARTSTAT=ACCEPTED:mailto:ada@example.com',
    'ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@example.com',
  ];

  // A weekly meeting since 2025. The email is about 15 September 2026, moved later.
  const SERIES = [
    'DTSTART:20250923T140000Z',
    'DTEND:20250923T141500Z',
    'RRULE:FREQ=WEEKLY',
    'SUMMARY:Huddle',
    ...GUESTS,
  ];
  const EMAILED = [
    'RECURRENCE-ID:20260915T140000Z',
    'DTSTART:20260915T150000Z',
    'DTEND:20260915T151500Z',
    'SUMMARY:Huddle',
    ...GUESTS,
  ];
  const SYNCED = [
    'RECURRENCE-ID:20260915T140000Z',
    'DTSTART:20260915T160000Z',
    'DTEND:20260915T161500Z',
    'SUMMARY:Huddle (as synced)',
    ...GUESTS,
  ];

  // Another moved week, listed first in the calendar copy.
  const OTHER_WEEK = [
    'RECURRENCE-ID:20260908T140000Z',
    'DTSTART:20260909T140000Z',
    'DTEND:20260909T141500Z',
    'SUMMARY:Huddle (another week)',
    ...GUESTS,
  ];

  // The header prints the day in the local zone, so the expectation is formed the same way
  // rather than spelled out: no instant falls on one date in every zone the suite may run in.
  const dayOf = (iso: string) => moment(iso).format('dddd, MMMM Do');

  let icsPath: string;
  let engine: ReturnType<typeof pimEngine>;
  let emailedIcs: string;

  // With `onOwnCalendar`, the copy sits on a writable calendar of ours, so an answer is written.
  function render(emailed: string[], calendarIcs: string, { onOwnCalendar = false } = {}) {
    icsPath = path.join(os.tmpdir(), `event-header-spec-${process.pid}.ics`);
    emailedIcs = vcalendar(emailed).replace('VERSION:2.0', 'VERSION:2.0\r\nMETHOD:REQUEST');
    fs.writeFileSync(icsPath, emailedIcs);
    spyOn(AttachmentStore, 'pathForFile').andReturn(icsPath);
    const mine = new Calendar({ id: 'cal-mine', accountId: 'a1', name: 'me@example.com' } as any);
    const synced = new Event({
      id: 'e1',
      accountId: 'a1',
      calendarId: onOwnCalendar ? mine.id : undefined,
      ics: calendarIcs,
    } as any);
    spyOn(Rx.Observable, 'fromQuery').andCallFake((query: { _klass: unknown }) =>
      Rx.Observable.just(
        query._klass === Event ? [synced] : query._klass === Calendar && onOwnCalendar ? [mine] : []
      )
    );
    const header = ReactTestUtils.renderIntoDocument(
      <EventHeader
        message={new Message({ id: 'm1', accountId: 'a1' })}
        file={new File({ id: 'f1', filename: 'invite.ics' })}
      />
    ) as unknown as EventHeader;
    const text = (className: string) =>
      ReactTestUtils.scryRenderedDOMComponentsWithClass(header, className)[0]?.textContent;
    // The link is rendered once the calendar copy has been applied.
    waitsFor(() => !!text('event-view-in-calendar'));
    return Object.assign(text, { header });
  }

  beforeEach(function () {
    engine = pimEngine();
    const account = new Account({ id: 'a1', emailAddress: 'me@example.com' });
    spyOn(AccountStore, 'accountForEmail').andCallFake((email: string) =>
      email === 'me@example.com' ? account : null
    );
    spyOn(AccountStore, 'accountForId').andReturn(account);
    spyOn(AccountStore, 'aliases').andReturn([]);
  });

  afterEach(function () {
    fs.unlinkSync(icsPath);
  });

  const answer = () => engine.callsTo('events.rsvp')[0];

  function click(text: ReturnType<typeof render>, label: string) {
    const button = ReactTestUtils.scryRenderedDOMComponentsWithClass(text.header, 'btn-rsvp').find(
      (b) => b.textContent === label
    );
    ReactTestUtils.Simulate.click(button);
    waitsFor(() => !!answer());
  }

  it("shows that occurrence from the calendar copy, not the series' first date", function () {
    const text = render(EMAILED, vcalendar(SERIES, OTHER_WEEK, SYNCED));
    runs(() => {
      expect(text('event-day')).toBe(dayOf('2026-09-15T16:00:00Z'));
      expect(text('event-title')).toBe('Huddle (as synced)');
    });
  });

  it('keeps what the email said when the calendar copy has no entry for it', function () {
    const text = render(EMAILED, vcalendar(SERIES));
    runs(() => {
      expect(text('event-day')).toBe(dayOf('2026-09-15T15:00:00Z'));
      expect(text('event-title')).toBe('Huddle');
    });
  });

  it('answers from the emailed occurrence, which the engine replies to alone', function () {
    const text = render(EMAILED, vcalendar(SERIES, OTHER_WEEK, SYNCED));
    runs(() => click(text, 'Decline'));
    runs(() => {
      expect(answer().ics).toBe(emailedIcs);
      expect(answer().messageId).toBe('m1');
      expect(answer().status).toBe('DECLINED');
      // Not on a calendar of ours: nothing is recorded.
      expect(answer().write).toBe(false);
      expect(answer().eventId).toBeUndefined();
    });
  });

  it('records the answer on our copy, where the engine finds the occurrence', function () {
    const text = render(EMAILED, vcalendar(SERIES, OTHER_WEEK), { onOwnCalendar: true });
    runs(() => click(text, 'Accept'));
    runs(() => {
      expect(answer().ics).toBe(emailedIcs);
      expect(answer().eventId).toBe('e1');
      expect(answer().status).toBe('ACCEPTED');
    });
  });

  it('still shows the calendar copy for an invitation to the whole series', function () {
    const text = render(
      SERIES,
      vcalendar([...SERIES.slice(0, 3), 'SUMMARY:Huddle (as synced)', ...GUESTS])
    );
    runs(() => {
      expect(text('event-day')).toBe(dayOf('2025-09-23T14:00:00Z'));
      expect(text('event-title')).toBe('Huddle (as synced)');
    });
  });
});

describe('EventHeader answering an invitation', function () {
  const GROUP_ORGANIZER = 'mailto:c_abc123@group.calendar.google.com';
  const INVITE = [
    'BEGIN:VCALENDAR',
    'VERSION:2.0',
    'PRODID:-//Test//Test//EN',
    'METHOD:REQUEST',
    'BEGIN:VEVENT',
    'UID:meeting-uid@example.com',
    'DTSTART:20260301T140000Z',
    'DTEND:20260301T150000Z',
    'SUMMARY:Kickoff',
    'DTSTAMP:20260101T000000Z',
    'ORGANIZER;CN=Ada:mailto:ada@example.com',
    'ATTENDEE;CN=Ada;ROLE=CHAIR;PARTSTAT=ACCEPTED:mailto:ada@example.com',
    'ATTENDEE;CN=Brian;ROLE=REQ-PARTICIPANT;RSVP=TRUE;PARTSTAT=NEEDS-ACTION:mailto:brian@example.com',
    'END:VEVENT',
    'END:VCALENDAR',
  ].join('\r\n');
  // Google's copy on a shared calendar: ORGANIZER rewritten to the calendar's own id.
  const SYNCED = INVITE.replace('METHOD:REQUEST\r\n', '').replace(
    'mailto:ada@example.com\r\nATTENDEE',
    `${GROUP_ORGANIZER}\r\nATTENDEE`
  );

  const account = new Account({ id: 'a1', emailAddress: 'brian@example.com' });
  const cal = (over: object) => new Calendar({ accountId: 'a1', ...over } as any);
  const mine = cal({ id: 'cal-mine', name: 'brian@example.com' });
  const shared = cal({ id: 'cal-shared', name: 'US On Call' });
  const theirs = cal({ id: 'cal-theirs', name: 'Team', ownership: 'other' });
  const holidays = cal({ id: 'cal-holidays', name: 'Holidays', readOnly: true });
  const synced = new Event({
    id: 'e-mine',
    accountId: 'a1',
    calendarId: mine.id,
    icsuid: 'meeting-uid@example.com',
    ics: SYNCED.replace('SUMMARY:Kickoff', 'SUMMARY:Kickoff (moved)'),
  } as any);
  const onRoom = synced.clone();
  onRoom.id = 'e-room';
  onRoom.calendarId = 'cal-room';
  onRoom.ics = SYNCED.replace('SUMMARY:Kickoff', 'SUMMARY:Kickoff (room copy)');

  let icsPath: string;
  let header: EventHeader;
  let queued: Task[];
  let engine: ReturnType<typeof pimEngine>;

  beforeEach(function () {
    engine = pimEngine();
    icsPath = path.join(os.tmpdir(), `event-header-rsvp-spec-${process.pid}.ics`);
    fs.writeFileSync(icsPath, INVITE);
    spyOn(AttachmentStore, 'pathForFile').andReturn(icsPath);
    spyOn(AccountStore, 'accountForId').andReturn(account);
    spyOn(AccountStore, 'aliases').andReturn([]);
    spyOn(AccountStore, 'accountForEmail').andCallFake((email: string) =>
      Utils.emailIsEquivalent(email, account.emailAddress) ? account : null
    );
    queued = [];
    spyOn(Actions, 'queueTask').andCallFake((task: Task) => queued.push(task));
    // A real error dialog would block the runner.
    spyOn(AppEnv, 'showErrorDialog');
  });

  afterEach(function () {
    fs.unlinkSync(icsPath);
  });

  function mount({
    events,
    calendars,
    from,
  }: {
    events: Event[];
    calendars: Calendar[];
    from?: string;
  }) {
    spyOn(Rx.Observable, 'fromQuery').andCallFake((query: { _klass: unknown }) =>
      Rx.Observable.just(query._klass === Event ? events : calendars)
    );
    header = ReactTestUtils.renderIntoDocument(
      <EventHeader
        message={
          new Message({
            id: 'm1',
            accountId: 'a1',
            from: from ? [new Contact({ email: from, name: 'Bo' })] : [],
          })
        }
        file={new File({ id: 'f1', filename: 'invite.ics' })}
      />
    ) as unknown as EventHeader;
    waitsFor(() => !!header.state.rsvp);
  }

  const answer = () => engine.callsTo('events.rsvp')[0];

  function click(label: string) {
    const button = ReactTestUtils.scryRenderedDOMComponentsWithClass(header, 'btn-rsvp').find(
      (n) => n.textContent === label
    );
    ReactTestUtils.Simulate.click(button);
    waitsFor(() => !!answer());
  }

  const destination = () =>
    ReactTestUtils.findRenderedDOMComponentWithClass(header, 'event-rsvp-destination');
  const title = () =>
    ReactTestUtils.findRenderedDOMComponentWithClass(header, 'event-title').textContent;

  it('shows the synced copy and answers from the mailed invitation, which names the organizer', function () {
    mount({ events: [synced], calendars: [mine] });
    runs(() => {
      expect(title()).toBe('Kickoff (moved)');
      click('Accept');
    });
    runs(() => {
      expect(answer().ics).toBe(INVITE);
      expect(answer().accountId).toBe('a1');
      expect(answer().messageId).toBe('m1');
      expect(answer().status).toBe('ACCEPTED');
      expect(answer().eventId).toBe('e-mine');
    });
  });

  it('records the answer on our own copy of several, and says so beforehand', function () {
    mount({
      events: [onRoom, synced],
      calendars: [cal({ id: 'cal-room', name: 'Boardroom' }), mine],
    });
    runs(() => {
      expect(title()).toBe('Kickoff (moved)');
      expect(destination().textContent).toBe('Your response will be saved to brian@example.com');
      click('Maybe');
    });
    runs(() => {
      expect(answer().eventId).toBe('e-mine');
      expect(answer().status).toBe('TENTATIVE');
    });
  });

  it('offers only calendars that could be ours when the invitation has not synced', function () {
    mount({ events: [], calendars: [theirs, holidays, shared, mine] });
    runs(() => {
      const picker = destination().querySelector('[data-slot="select-trigger"]');
      expect(picker.getAttribute('aria-label')).toBe('Calendar to add this event to');
      expect(picker.textContent).toBe(mine.name);
      ReactTestUtils.Simulate.keyDown(picker, { key: 'Enter' });
      const items = Array.from(
        getPortalContainer().querySelectorAll(
          '[data-slot="select-content"][data-state="open"] [data-slot="select-item"]'
        )
      );
      expect(items.map((o) => o.textContent)).toEqual(['US On Call', 'brian@example.com']);
      expect(items[1].getAttribute('aria-selected')).toBe('true');
      ReactTestUtils.Simulate.keyDown(items[1], { key: 'Enter' });
      click('Accept');
    });
    runs(() => {
      expect(answer().calendarId).toBe(mine.id);
      expect(answer().eventId).toBeUndefined();
    });
  });

  it('names where an acceptance would go when the invitation is on no calendar', function () {
    mount({ events: [], calendars: [mine] });
    runs(() => {
      expect(Array.from(destination().querySelectorAll('span')).map((n) => n.textContent)).toEqual([
        'Accepting will add this event to',
        'brian@example.com',
      ]);
      expect(destination().querySelector('[data-slot="select-trigger"]')).toBe(null);
      expect(
        ReactTestUtils.scryRenderedDOMComponentsWithClass(header, 'event-view-in-calendar').length
      ).toBe(0);
      // The engine adds it only for an acceptance (plan.rs rsvp_adds_an_invitation_...).
      click('Decline');
    });
    runs(() => {
      expect(answer().status).toBe('DECLINED');
      expect(answer().calendarId).toBe(mine.id);
    });
  });

  it('still emails the reply when every copy is read-only, and says nothing is recorded', function () {
    const onHolidays = synced.clone();
    onHolidays.calendarId = holidays.id;
    mount({ events: [onHolidays], calendars: [holidays] });
    runs(() => {
      expect(title()).toBe('Kickoff (moved)');
      expect(destination().textContent).toContain('only be emailed to the organizer');
      click('Accept');
    });
    runs(() => {
      expect(answer().write).toBe(false);
      expect(answer().eventId).toBeUndefined();
    });
  });

  it('says so when the engine finds we are not invited', function () {
    engine.answers['events.rsvp'] = () => {
      throw refusal('not-invited');
    };
    mount({ events: [synced], calendars: [mine] });
    runs(() => click('Accept'));
    waitsFor(() => (AppEnv.showErrorDialog as jasmine.Spy).callCount > 0);
    runs(() => {
      expect((AppEnv.showErrorDialog as jasmine.Spy).mostRecentCall.args[0]).toContain(
        'attendee list'
      );
      expect(header.state.inflight).toBe(undefined);
    });
  });

  it('offers no RSVP on an invitation that lists a group rather than us', function () {
    fs.writeFileSync(
      icsPath,
      INVITE.replace('mailto:brian@example.com', 'mailto:team@example.com')
    );
    mount({ events: [], calendars: [mine] });
    runs(() => {
      expect(ReactTestUtils.scryRenderedDOMComponentsWithClass(header, 'btn-rsvp').length).toBe(0);
      expect(
        ReactTestUtils.findRenderedDOMComponentWithClass(header, 'event-no-rsvp').textContent
      ).toContain("isn't listed as a guest");
    });
  });

  it('loads the invitation once the engine has downloaded it', function () {
    fs.unlinkSync(icsPath);
    spyOn(Rx.Observable, 'fromQuery').andReturn(Rx.Observable.just([]));
    const container = document.createElement('div');
    const render = (message: Message) =>
      ReactDOM.render(
        <EventHeader message={message} file={new File({ id: 'f1', filename: 'invite.ics' })} />,
        container
      ) as unknown as EventHeader;
    header = render(new Message({ id: 'm1', accountId: 'a1' }));
    waits(50);
    runs(() => {
      expect(header.state.inviteIcs).toBe(undefined);
      fs.writeFileSync(icsPath, INVITE);
      header = render(new Message({ id: 'm1', accountId: 'a1' }));
    });
    waitsFor(() => !!header.state.inviteIcs);
    runs(() => {
      expect(title()).toBe('Kickoff');
      ReactDOM.unmountComponentAtNode(container);
    });
  });

  describe('conflicts', function () {
    const conflict = (title: string, start: string, end: string): EventConflictJSON => ({
      eventId: title,
      calendarId: mine.id,
      icsuid: `${title}@test`,
      title,
      start: unixOf(start),
      end: unixOf(end),
    });
    const conflictLines = () =>
      ReactTestUtils.scryRenderedDOMComponentsWithClass(header, 'event-conflict').map(
        (n) => n.textContent
      );
    const lookUp = () => engine.callsTo('events.expand').find((p) => p.conflicts);
    const SERIES = INVITE.replace(
      'DTSTART:20260301T140000Z\r\nDTEND:20260301T150000Z',
      'DTSTART:20260301T140000Z\r\nDTEND:20260301T150000Z\r\nRRULE:FREQ=WEEKLY'
    );
    // The series' next occurrence, as the engine finds it.
    const NEXT = { start: unixOf('20261025T140000Z'), end: unixOf('20261025T150000Z') };

    beforeEach(function () {
      const original = AppEnv.config.get;
      spyOn(AppEnv.config, 'get').andCallFake((key: string) =>
        key === 'core.calendar.disabledCalendars' ? ['cal-off'] : original.call(AppEnv.config, key)
      );
      engine.expand.conflicts = () => [conflict('Standup', '20260301T143000Z', '20260301T150000Z')];
    });

    it('asks for what overlaps the invitation on the calendars that are our busy time', function () {
      const off = cal({ id: 'cal-off', name: 'Side project' });
      mount({ events: [], calendars: [mine, holidays, theirs, off] });
      waitsFor(() => conflictLines().length > 0);
      runs(() => {
        expect(lookUp().calendarIds).toEqual([mine.id]);
        expect(lookUp().conflicts).toEqual({ excludeIcsuid: 'meeting-uid@example.com' });
        expect([lookUp().start, lookUp().end]).toEqual([
          unixOf('20260301T140000Z'),
          unixOf('20260301T150000Z'),
        ]);
        expect(
          ReactTestUtils.findRenderedDOMComponentWithClass(header, 'event-conflicts-title')
            .textContent
        ).toBe('Conflicts with an event on your calendar');
        // The header shows the invitation's day, so the line gives only times.
        expect(conflictLines()).toEqual(['Standup (8:30 am - 9:00 am)']);
      });
    });

    it('checks the next occurrence of a recurring invitation, not the one it began with', function () {
      fs.writeFileSync(icsPath, SERIES);
      engine.expand.upcoming = (ics) => (ics === SERIES ? { ...NEXT, isAllDay: false } : null);
      engine.expand.conflicts = () => [
        conflict('This week', '20261025T140000Z', '20261025T150000Z'),
      ];
      mount({ events: [], calendars: [mine] });
      waitsFor(() => conflictLines().length > 0);
      runs(() => {
        expect(header.state.conflictWindow).toEqual(NEXT);
        expect([lookUp().start, lookUp().end]).toEqual([NEXT.start, NEXT.end]);
        // The header shows the series' first date, so each line names the day it checked.
        expect(conflictLines()[0]).toContain(
          `(${moment.unix(NEXT.start).format('dddd, MMMM Do')}, `
        );
      });
    });

    it('shows no conflicts on a reply, where we picked the slot ourselves', function () {
      fs.writeFileSync(icsPath, INVITE.replace('METHOD:REQUEST', 'METHOD:REPLY'));
      mount({ events: [], calendars: [mine] });
      waitsFor(() => header.state.conflicts && header.state.conflicts.length > 0);
      runs(() => {
        expect(
          ReactTestUtils.scryRenderedDOMComponentsWithClass(header, 'event-conflicts').length
        ).toBe(0);
      });
    });

    it('does not bother with conflicts on a cancellation', function () {
      fs.writeFileSync(icsPath, INVITE.replace('METHOD:REQUEST', 'METHOD:CANCEL'));
      mount({ events: [], calendars: [mine] });
      waitsFor(() => header.state.conflicts && header.state.conflicts.length > 0);
      runs(() => {
        expect(
          ReactTestUtils.scryRenderedDOMComponentsWithClass(header, 'event-conflicts').length
        ).toBe(0);
      });
    });

    it('follows the calendar copy when it has moved the next occurrence of a series', function () {
      fs.writeFileSync(icsPath, SERIES);
      const copy = synced.clone();
      copy.ics = SERIES.replace('METHOD:REQUEST\r\n', '').replace('Kickoff', 'Kickoff (copy)');
      const later = { start: NEXT.start + 7200, end: NEXT.end + 7200, isAllDay: false };
      engine.expand.upcoming = (ics) =>
        ics === copy.ics ? later : ics === SERIES ? { ...NEXT, isAllDay: false } : null;
      mount({ events: [copy], calendars: [mine] });
      waitsFor(() => !!header.state.conflictWindow);
      runs(() => {
        expect(header.state.conflictWindow).toEqual({ start: later.start, end: later.end });
      });
    });

    it('checks the occurrence an email is about where the calendar copy now has it', function () {
      // The email names the 8 March occurrence of a weekly series; our copy has moved it to 16:00.
      const emailed = INVITE.replace(
        'DTSTART:20260301T140000Z\r\nDTEND:20260301T150000Z',
        'RECURRENCE-ID:20260308T140000Z\r\nDTSTART:20260308T140000Z\r\nDTEND:20260308T150000Z'
      );
      fs.writeFileSync(icsPath, emailed);
      const series = synced.clone();
      series.ics = SYNCED.replace(
        'DTEND:20260301T150000Z',
        'DTEND:20260301T150000Z\r\nRRULE:FREQ=WEEKLY'
      ).replace(
        'END:VCALENDAR',
        [
          'BEGIN:VEVENT',
          'UID:meeting-uid@example.com',
          'RECURRENCE-ID:20260308T140000Z',
          'DTSTART:20260308T160000Z',
          'DTEND:20260308T170000Z',
          'SUMMARY:Kickoff (moved later)',
          'DTSTAMP:20260101T000000Z',
          'END:VEVENT',
          'END:VCALENDAR',
        ].join('\r\n')
      );
      engine.expand.conflicts = () => [
        conflict('At the moved time', '20260308T163000Z', '20260308T170000Z'),
      ];
      mount({ events: [series], calendars: [mine] });
      waitsFor(() => conflictLines().length > 0);
      runs(() => {
        expect(title()).toBe('Kickoff (moved later)');
        const window = {
          start: Date.UTC(2026, 2, 8, 16) / 1000,
          end: Date.UTC(2026, 2, 8, 17) / 1000,
        };
        expect(header.state.conflictWindow).toEqual(window);
        expect({ start: lookUp().start, end: lookUp().end }).toEqual(window);
        expect(conflictLines().map((l) => l.split(' (')[0])).toEqual(['At the moved time']);
      });
    });

    it('says nothing when the slot is free', function () {
      engine.expand.conflicts = () => [];
      mount({ events: [], calendars: [mine] });
      waitsFor(() => !!lookUp());
      runs(() => {
        expect(
          ReactTestUtils.scryRenderedDOMComponentsWithClass(header, 'event-conflicts').length
        ).toBe(0);
      });
    });
  });

  describe('a counter-proposal from a guest', function () {
    // Our copy of a meeting we organize, with Bo as a guest.
    const OURS = INVITE.replace('METHOD:REQUEST\r\n', '')
      .replace(
        'ORGANIZER;CN=Ada:mailto:ada@example.com',
        'ORGANIZER;CN=Brian:mailto:brian@example.com'
      )
      .replace(
        'ATTENDEE;CN=Ada;ROLE=CHAIR;PARTSTAT=ACCEPTED:mailto:ada@example.com',
        'ATTENDEE;CN=Brian;ROLE=CHAIR;PARTSTAT=ACCEPTED:mailto:brian@example.com'
      )
      .replace(
        'ATTENDEE;CN=Brian;ROLE=REQ-PARTICIPANT;RSVP=TRUE;PARTSTAT=NEEDS-ACTION:mailto:brian@example.com',
        'ATTENDEE;CN=Bo;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION:mailto:bo@example.com'
      );
    const COUNTER = OURS.replace('BEGIN:VEVENT', 'METHOD:COUNTER\r\nBEGIN:VEVENT')
      .replace('DTSTART:20260301T140000Z', 'DTSTART:20260301T160000Z')
      .replace('DTEND:20260301T150000Z', 'DTEND:20260301T170000Z');
    const MOVED = OURS.replace('SUMMARY:Kickoff', 'SUMMARY:Kickoff\r\nSEQUENCE:1');
    const ours = new Event({
      id: 'e-ours',
      accountId: 'a1',
      calendarId: mine.id,
      icsuid: 'meeting-uid@example.com',
      recurrenceStart: Date.UTC(2026, 2, 1, 14) / 1000,
      recurrenceEnd: Date.UTC(2026, 2, 1, 15) / 1000,
      ics: OURS,
    } as any);
    const moveButton = () =>
      ReactTestUtils.scryRenderedDOMComponentsWithClass(header, 'btn-large').find(
        (n) => n.textContent === 'Move event to this time'
      );
    const update = () => engine.callsTo('events.update')[0];

    beforeEach(function () {
      fs.writeFileSync(icsPath, COUNTER);
      engine.answers['events.update'] = (params) => ({
        writes: [{ kind: 'update', eventId: params.eventId, ics: MOVED }],
      });
    });

    it('shows the proposed time and moves the event to it, undoably', function () {
      mount({ events: [ours], calendars: [mine], from: 'bo@example.com' });
      runs(() => {
        expect(header.state.icsEvent.startDate.toJSDate().toISOString()).toBe(
          '2026-03-01T16:00:00.000Z'
        );
        expect(destination().textContent).toBe('Your response will be saved to brian@example.com');
        ReactTestUtils.Simulate.click(moveButton());
      });
      waitsFor(() => queued.length > 0);
      runs(() => {
        expect(update().eventId).toBe('e-ours');
        expect(update().scope).toBe('all');
        expect(update().occurrenceStart).toBeUndefined();
        expect(update().patch).toEqual({
          start: Date.UTC(2026, 2, 1, 16) / 1000,
          end: Date.UTC(2026, 2, 1, 17) / 1000,
          isAllDay: false,
        });
        const write = queued[0] as SyncbackEventTask;
        expect(write instanceof SyncbackEventTask).toBe(true);
        expect(write.event.id).toBe('e-ours');
        expect(write.event.ics).toBe(MOVED);
        expect(write.undoData.ics).toBe(OURS);
      });
    });

    it('moves only the occurrence a counter names', function () {
      const series = ours.clone();
      series.ics = OURS.replace(
        'DTEND:20260301T150000Z',
        'DTEND:20260301T150000Z\r\nRRULE:FREQ=WEEKLY'
      );
      fs.writeFileSync(
        icsPath,
        COUNTER.replace(
          'DTSTART:20260301T160000Z',
          'RECURRENCE-ID:20260308T140000Z\r\nDTSTART:20260308T160000Z'
        ).replace('DTEND:20260301T170000Z', 'DTEND:20260308T170000Z')
      );
      mount({ events: [series], calendars: [mine], from: 'bo@example.com' });
      runs(() => ReactTestUtils.Simulate.click(moveButton()));
      waitsFor(() => queued.length > 0);
      runs(() => {
        expect(update().scope).toBe('this');
        expect(update().occurrenceStart).toBe(Date.UTC(2026, 2, 8, 14) / 1000);
        expect(update().patch.start).toBe(Date.UTC(2026, 2, 8, 16) / 1000);
      });
    });

    it('withholds the move when a counter for a series names no occurrence', function () {
      const series = ours.clone();
      series.ics = OURS.replace(
        'DTEND:20260301T150000Z',
        'DTEND:20260301T150000Z\r\nRRULE:FREQ=WEEKLY'
      );
      mount({ events: [series], calendars: [mine], from: 'bo@example.com' });
      runs(() => {
        expect(moveButton()).toBe(undefined);
        expect(destination().textContent).toContain('which occurrence');
      });
    });

    it('keeps an all-day series on dates when moving one occurrence', function () {
      const ALL_DAY_OURS = OURS.replace(
        'DTSTART:20260301T140000Z',
        'DTSTART;VALUE=DATE:20260301'
      ).replace('DTEND:20260301T150000Z', 'DTEND;VALUE=DATE:20260302\r\nRRULE:FREQ=WEEKLY');
      const allDaySeries = ours.clone();
      allDaySeries.ics = ALL_DAY_OURS;
      fs.writeFileSync(
        icsPath,
        ALL_DAY_OURS.replace('BEGIN:VEVENT', 'METHOD:COUNTER\r\nBEGIN:VEVENT').replace(
          'DTSTART;VALUE=DATE:20260301\r\nDTEND;VALUE=DATE:20260302\r\nRRULE:FREQ=WEEKLY',
          'RECURRENCE-ID;VALUE=DATE:20260308\r\nDTSTART;VALUE=DATE:20260309\r\nDTEND;VALUE=DATE:20260310'
        )
      );
      mount({ events: [allDaySeries], calendars: [mine], from: 'bo@example.com' });
      runs(() => ReactTestUtils.Simulate.click(moveButton()));
      waitsFor(() => queued.length > 0);
      runs(() => {
        expect(update().scope).toBe('this');
        expect(update().patch.isAllDay).toBe(true);
      });
    });

    it('refuses when our copy says somebody else organizes the meeting', function () {
      const theirs = ours.clone();
      theirs.ics = OURS.replace(
        'ORGANIZER;CN=Brian:mailto:brian@example.com',
        'ORGANIZER;CN=Ada:mailto:ada@example.com'
      );
      mount({ events: [theirs], calendars: [mine], from: 'bo@example.com' });
      runs(() => {
        expect(moveButton()).toBe(undefined);
        expect(destination().textContent).toContain('only its organizer can move it');
      });
    });

    it('refuses a proposal from someone who is not a guest', function () {
      mount({ events: [ours], calendars: [mine], from: 'stranger@example.net' });
      runs(() => {
        expect(moveButton()).toBe(undefined);
        expect(destination().textContent).toContain('is not a guest');
      });
    });

    it('refuses when the meeting is on no calendar we can edit', function () {
      mount({ events: [], calendars: [mine], from: 'bo@example.com' });
      runs(() => {
        expect(moveButton()).toBe(undefined);
        expect(destination().textContent).toContain("isn't on a calendar you can edit");
      });
    });

    it('reports a proposal the engine cannot apply instead of crashing', function () {
      engine.answers['events.update'] = () => {
        throw refusal('invalid-params');
      };
      mount({ events: [ours], calendars: [mine], from: 'bo@example.com' });
      runs(() => ReactTestUtils.Simulate.click(moveButton()));
      waitsFor(() => (AppEnv.showErrorDialog as jasmine.Spy).callCount > 0);
      runs(() => expect(queued.length).toBe(0));
    });
  });
});
