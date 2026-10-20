// The calendar's answers and counter-proposals go to the engine (events.rsvp), which builds the
// PARTSTAT edit, the REPLY and the COUNTER (crates/ms-pim plan.rs and its tests).
import { AccountStore, Actions, DatabaseStore, Event } from 'mailspring-exports';
import {
  occurrenceSlot,
  offerCounterInsteadOfMove,
  openProposeNewTimePopover,
  proposeNewTimeForCalendarEvent,
  respondToCalendarEvent,
} from '../../../src/features/main-calendar/event/rsvp';
import { parseCalendarDate } from '../../../src/utils/calendar-date';
import { fakePimEngine, refusal } from '../../fake-pim-engine';

const ME = 'brian@example.com';
const START = Date.UTC(2026, 2, 10, 14) / 1000;

const SERIES_ICS = [
  'BEGIN:VCALENDAR',
  'VERSION:2.0',
  'PRODID:-//Test//Test//EN',
  'BEGIN:VEVENT',
  'UID:series@test',
  'DTSTART:20260303T140000Z',
  'DTEND:20260303T150000Z',
  'RRULE:FREQ=WEEKLY',
  'SUMMARY:Design Review',
  'DTSTAMP:20260101T000000Z',
  'ORGANIZER;CN=Ada:mailto:ada@example.com',
  'ATTENDEE;CN=Ada;ROLE=CHAIR;PARTSTAT=ACCEPTED:mailto:ada@example.com',
  `ATTENDEE;CN=Me;ROLE=REQ-PARTICIPANT;RSVP=TRUE;PARTSTAT=NEEDS-ACTION:mailto:${ME}`,
  'END:VEVENT',
  'END:VCALENDAR',
].join('\r\n');

function occurrence(overrides: any = {}) {
  return {
    id: 'event-1-e1773151200',
    accountId: 'acct-1',
    calendarId: 'cal-1',
    title: 'Design Review',
    isAllDay: false,
    start: START,
    end: START + 3600,
    startDate: parseCalendarDate('2026-03-10'),
    endDate: parseCalendarDate('2026-03-10'),
    isRecurring: true,
    isException: false,
    organizer: { email: 'ada@example.com' },
    isMine: false,
    attendees: [
      { email: 'ada@example.com', name: 'Ada', partstat: 'ACCEPTED' },
      { email: ME, name: 'Me', partstat: 'NEEDS-ACTION' },
    ],
    ...overrides,
  } as any;
}

describe('occurrenceSlot', function () {
  it('names a regular occurrence by its start', function () {
    expect(occurrenceSlot(occurrence())).toBe(START);
  });

  it('names a moved occurrence by where it was, not where it is', function () {
    const moved = occurrence({ isException: true, recurrenceIdStart: START - 86400 });
    expect(occurrenceSlot(moved)).toBe(START - 86400);
  });
});

describe('answering and countering from the calendar', function () {
  let event: Event;
  let dialog: jasmine.Spy;
  let engine: ReturnType<typeof fakePimEngine>;

  beforeEach(function () {
    engine = fakePimEngine();
    engine.answers['events.rsvp'] = () => ({ email: ME, replyTaskId: 't-reply' });
    event = new Event({
      id: 'event-1',
      accountId: 'acct-1',
      calendarId: 'cal-1',
      icsuid: 'series@test',
      ics: SERIES_ICS,
    } as any);
    spyOn(DatabaseStore, 'find').andCallFake(() => Promise.resolve(event));
    spyOn(AccountStore, 'accountForEmail').andCallFake((email: string) =>
      email.toLowerCase() === ME ? { id: 'acct-1', emailAddress: ME } : null
    );
    spyOn(AccountStore, 'accountForId').andReturn({ id: 'acct-1', emailAddress: ME });
    spyOn(AppEnv, 'showErrorDialog');
    // The recurring-event question; 0 = this occurrence, 1 = all occurrences, 2 = cancel.
    dialog = spyOn(require('@electron/remote').dialog, 'showMessageBoxSync').andReturn(1);
  });

  describe('proposeNewTimeForCalendarEvent', function () {
    const proposal = {
      start: new Date('2026-03-10T16:00:00Z'),
      end: new Date('2026-03-10T17:00:00Z'),
      comment: 'Clashes with standup',
    };

    it('asks the engine for a COUNTER naming the occurrence', async function () {
      await proposeNewTimeForCalendarEvent(occurrence(), proposal);
      const [params] = engine.callsTo('events.rsvp');
      expect(params.accountId).toBe('acct-1');
      expect(params.eventId).toBe('event-1');
      expect(params.counter).toEqual({
        start: Date.UTC(2026, 2, 10, 16) / 1000,
        end: Date.UTC(2026, 2, 10, 17) / 1000,
      });
      expect(params.occurrenceStart).toBe(START);
      expect(params.comment).toBe('Clashes with standup');
      expect(params.subject).toBe('New time proposed: Design Review');
    });

    it('names no occurrence for an event that does not repeat', async function () {
      await proposeNewTimeForCalendarEvent(occurrence({ isRecurring: false }), proposal);
      expect(engine.callsTo('events.rsvp')[0].occurrenceStart).toBeUndefined();
    });

    it('explains instead of sending when we are not a guest', async function () {
      await proposeNewTimeForCalendarEvent(
        occurrence({ attendees: [{ email: 'ada@example.com', partstat: 'ACCEPTED' }] }),
        proposal
      );
      expect(engine.calls.length).toBe(0);
      expect((AppEnv.showErrorDialog as jasmine.Spy).mostRecentCall.args[0]).toContain(
        'guest list'
      );
    });

    it('explains when the engine finds no organizer to send it to', async function () {
      engine.answers['events.rsvp'] = () => {
        throw refusal('no-organizer');
      };
      await proposeNewTimeForCalendarEvent(occurrence(), proposal);
      expect((AppEnv.showErrorDialog as jasmine.Spy).mostRecentCall.args[0]).toContain(
        'no organizer'
      );
    });
  });

  describe('openProposeNewTimePopover', function () {
    it('opens the picker on the occurrence, dates only for an all-day one', function () {
      const openPopover = spyOn(Actions, 'openPopover');
      openProposeNewTimePopover(occurrence());
      let element = openPopover.mostRecentCall.args[0];
      expect(element.props.start).toBe(START);
      expect(element.props.end).toBe(START + 3600);
      expect(element.props.isAllDay).toBe(false);

      openProposeNewTimePopover(occurrence({ isAllDay: true, start: undefined, end: undefined }));
      element = openPopover.mostRecentCall.args[0];
      expect(element.props.isAllDay).toBe(true);
      expect(element.props.end - element.props.start).toBe(86400);
    });
  });

  describe('respondToCalendarEvent', function () {
    it('answers on our copy; the engine writes it and emails the organizer', async function () {
      await respondToCalendarEvent(occurrence(), 'ACCEPTED');
      const [params] = engine.callsTo('events.rsvp');
      expect(params.accountId).toBe('acct-1');
      expect(params.eventId).toBe('event-1');
      expect(params.status).toBe('ACCEPTED');
      expect(params.occurrenceStart).toBeUndefined();
    });

    it('asks whether the answer is for this occurrence or the whole series', async function () {
      await respondToCalendarEvent(occurrence(), 'ACCEPTED');
      expect(dialog.callCount).toBe(1);
      const options = dialog.mostRecentCall.args[0];
      expect(options.buttons).toEqual(['This occurrence only', 'All occurrences', 'Cancel']);
      expect(options.message).toContain('Design Review');
      expect(options.detail).toContain('answer only this occurrence');
    });

    it('answers only the clicked occurrence when asked to', async function () {
      dialog.andReturn(0);
      await respondToCalendarEvent(occurrence(), 'ACCEPTED');
      expect(engine.callsTo('events.rsvp')[0].occurrenceStart).toBe(START);
    });

    it('does nothing when the question is dismissed', async function () {
      dialog.andReturn(2);
      await respondToCalendarEvent(occurrence(), 'ACCEPTED');
      expect(engine.calls.length).toBe(0);
    });

    it('does not ask about an event that does not repeat', async function () {
      await respondToCalendarEvent(occurrence({ isRecurring: false }), 'ACCEPTED');
      expect(dialog.callCount).toBe(0);
      expect(engine.callsTo('events.rsvp').length).toBe(1);
    });

    it('explains when the engine does not find us on the guest list', async function () {
      engine.answers['events.rsvp'] = () => {
        throw refusal('not-invited');
      };
      await respondToCalendarEvent(occurrence({ isRecurring: false }), 'ACCEPTED');
      expect((AppEnv.showErrorDialog as jasmine.Spy).mostRecentCall.args[0]).toContain(
        'guest list'
      );
    });
  });
});

describe('offerCounterInsteadOfMove', function () {
  const remote = require('@electron/remote');

  it('offers the counter-proposal and opens it when taken up', function () {
    const dialog = spyOn(remote.dialog, 'showMessageBoxSync').andReturn(0);
    const openPopover = spyOn(Actions, 'openPopover');
    offerCounterInsteadOfMove(occurrence());
    expect(dialog.mostRecentCall.args[0].buttons[0]).toBe('Propose a new time');
    expect(dialog.mostRecentCall.args[0].message).toContain('Design Review');
    expect(openPopover.mostRecentCall.args[0].props.start).toBe(START);
  });

  it('does nothing more when cancelled', function () {
    spyOn(remote.dialog, 'showMessageBoxSync').andReturn(1);
    const openPopover = spyOn(Actions, 'openPopover');
    offerCounterInsteadOfMove(occurrence());
    expect(openPopover).not.toHaveBeenCalled();
  });
});
