import { Actions } from 'mailspring-exports';
import moment from 'moment';
import { Event as MailspringEvent } from '../../../src/data/models/event';
import { CalendarEventPopover } from '../../../src/features/main-calendar/event/popover';
import { TimedOccurrence } from '../../../src/features/main-calendar/data-source';
import { fakePimEngine } from '../../fake-pim-engine';

// A one-hour meeting, not recurring, so the save sends absolute times.
const START = moment('2026-03-10 10:00', 'YYYY-MM-DD HH:mm').unix();
const END = moment('2026-03-10 11:00', 'YYYY-MM-DD HH:mm').unix();

const TIMED_ICS = `BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//Test//EN
BEGIN:VEVENT
UID:timed@test
DTSTART:20260310T160000Z
DTEND:20260310T170000Z
SUMMARY:Standup
DTSTAMP:20260101T000000Z
SEQUENCE:0
END:VEVENT
END:VCALENDAR`;

function makeEvent() {
  return new MailspringEvent({
    id: 'event-1',
    accountId: 'account-1',
    calendarId: 'calendar-1',
    ics: TIMED_ICS,
    recurrenceStart: START,
    recurrenceEnd: END,
  } as any);
}

function makeOccurrence(): TimedOccurrence {
  return {
    id: 'event-1-e0',
    accountId: 'account-1',
    calendarId: 'calendar-1',
    title: 'Standup',
    location: '',
    description: '',
    isAllDay: false,
    isCancelled: false,
    isPending: false,
    isException: false,
    isRecurring: false,
    organizer: null,
    attendees: [],
    start: START,
    end: END,
  } as TimedOccurrence;
}

async function openEditor() {
  const popover: any = new CalendarEventPopover({
    event: makeOccurrence(),
    onEdit: () => {},
    onDelete: () => {},
  } as any);
  popover.setState = (update: object) => Object.assign(popover.state, update);
  await popover.onEdit();
  return popover;
}

// The toggle leaves state.start and state.end at their wall-clock values; shiftEndWithStart's
// all-day branch keeps the span, and the engine writes the DATEs, rolling the last second over
// to the next day (crates/ms-pim's desktop corpus has these rows with the DATEs written). The
// days the patch covers are checked here: from the start's day to the day of its last second.
describe('CalendarEventPopover all-day toggle', function () {
  let engine: ReturnType<typeof fakePimEngine>;
  const patch = () => engine.callsTo('events.update')[0].patch;
  const days = () => [
    moment.unix(patch().start).format('YYYYMMDD'),
    moment
      .unix(patch().end - 1)
      .add(1, 'day')
      .format('YYYYMMDD'),
  ];

  beforeEach(function () {
    engine = fakePimEngine();
    engine.answers['events.update'] = () => ({ writes: [] });
    spyOn(Actions, 'queueTask');
  });

  // Each row toggles all-day on from a timed event, then moves the start, and asserts the
  // written span. Spring forward is 2026-03-08, so two of these cross it.
  [
    {
      what: 'moving the start forward',
      to: '2026-03-12 10:00',
      dtstart: '20260312',
      dtend: '20260313',
    },
    {
      what: 'moving the start backward',
      to: '2026-03-04 10:00',
      dtstart: '20260304',
      dtend: '20260305',
    },
    {
      what: 'moving back across spring forward',
      to: '2026-03-06 10:00',
      dtstart: '20260306',
      dtend: '20260307',
    },
    {
      what: 'landing on the spring-forward day',
      to: '2026-03-08 10:00',
      dtstart: '20260308',
      dtend: '20260309',
    },
    {
      what: 'not moving the start at all',
      to: '2026-03-10 10:00',
      dtstart: '20260310',
      dtend: '20260311',
    },
  ].forEach(({ what, to, dtstart, dtend }) => {
    it(`writes one whole day when ${what}`, async function () {
      const popover = await openEditor();

      popover.updateField('allDay', true);
      popover.updateStart(moment(to, 'YYYY-MM-DD HH:mm').unix());
      await popover._saveAllOccurrences(makeEvent(), false);

      expect(patch().isAllDay).toBe(true);
      expect(days()).toEqual([dtstart, dtend]);
    });
  });

  it('keeps a three-day span three days long', async function () {
    const popover = await openEditor();

    popover.updateEnd(moment('2026-03-12 11:00', 'YYYY-MM-DD HH:mm').unix());
    popover.updateField('allDay', true);
    popover.updateStart(moment('2026-03-17 10:00', 'YYYY-MM-DD HH:mm').unix());
    await popover._saveAllOccurrences(makeEvent(), false);

    expect(days()).toEqual(['20260317', '20260320']);
  });
});
