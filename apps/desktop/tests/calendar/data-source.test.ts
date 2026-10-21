// The engine expands the occurrences (events.expand); its rules, the desktop corpus they were
// checked against and the fixture calendar are crates/tern-pim's tests. Here: what the calendar
// asks for, and the helpers that read the answer.
import { Event as EventModel } from '../../src/data/models/event';
import { Calendar } from '../../src/data/models/calendar';
import { DatabaseStore } from 'mailspring-exports';
import {
  CalendarDataSource,
  occurrencesForEvents,
  eventCoversDate,
  isEventSelected,
  focusedEventInfoForEvents,
  EventOccurrence,
} from '../../src/features/main-calendar/data-source';
import { parseCalendarDate } from '../../src/data/calendar/calendar-date';
import { fakePimEngine, flush } from '../support/fake-pim-engine';

const day = (iso: string) => parseCalendarDate(iso);

function allDay(startIso: string, endIso: string): EventOccurrence {
  return {
    id: 'e-1-e0',
    accountId: 'a',
    calendarId: 'c',
    title: 'Holiday',
    location: '',
    description: '',
    isCancelled: false,
    isPending: false,
    isMine: true,
    isException: false,
    isRecurring: false,
    organizer: null,
    attendees: [],
    startDate: day(startIso),
    endDate: day(endIso),
    isAllDay: true,
  };
}

describe('eventCoversDate', function () {
  it('covers each day of a multi-day all-day span and neither neighbour', function () {
    const occ = allDay('2026-06-22', '2026-06-24');
    expect(
      ['2026-06-21', '2026-06-22', '2026-06-23', '2026-06-24', '2026-06-25'].map((iso) =>
        eventCoversDate(occ, day(iso))
      )
    ).toEqual([false, true, true, true, false]);
  });

  it('covers exactly one day for a one-day all-day event', function () {
    const occ = allDay('2026-06-22', '2026-06-22');
    expect(eventCoversDate(occ, day('2026-06-22'))).toBe(true);
    expect(eventCoversDate(occ, day('2026-06-23'))).toBe(false);
  });
});

describe('isEventSelected', function () {
  const occ = (id: string) => ({ id }) as EventOccurrence;

  it('matches by id even when the object identity differs', function () {
    // Selection is captured at click time; a data refresh gives the occurrence a fresh object,
    // so an identity check would drop the highlight. This is the week-view bug this guards.
    const selected = [occ('evt-1'), occ('evt-2')];
    expect(isEventSelected(selected, occ('evt-1'))).toBe(true);
  });

  it('does not match a different id', function () {
    expect(isEventSelected([occ('evt-1')], occ('evt-9'))).toBe(false);
  });

  it('is false for an empty selection', function () {
    expect(isEventSelected([], occ('evt-1'))).toBe(false);
  });
});

describe('CalendarDataSource', function () {
  let engine: ReturnType<typeof fakePimEngine>;
  let calendars: Calendar[];

  beforeEach(function () {
    engine = fakePimEngine();
    calendars = [
      new Calendar({ id: 'work', accountId: 'a' } as any),
      new Calendar({ id: 'holidays', accountId: 'a' } as any),
    ];
    spyOn(DatabaseStore, 'run').andCallFake(() => Promise.resolve(calendars));
    engine.answers['events.expand'] = (params) => ({
      occurrences: (params.calendarIds || []).map((id: string) => ({
        ...allDay('2026-06-22', '2026-06-22'),
        calendarId: id,
      })),
    });
  });

  it('expands the shown range on the calendars that are switched on', async function () {
    const seen: EventOccurrence[][] = [];
    const source = new CalendarDataSource();
    source.buildObservable({ startUnix: 100, endUnix: 200, disabledCalendars: ['holidays'] });
    const sub = source.subscribe(({ events }) => seen.push(events));
    // Rx delivers a promise's value on its own scheduler, which the faked clock doesn't drive.
    for (let i = 0; i < 20 && !seen.length; i++) {
      advanceClock(10);
      await new Promise((resolve) => setImmediate(resolve));
    }
    sub.dispose();
    // Runs the pool's cleanup timer, which a later spec's subscriptions would otherwise wait on.
    advanceClock(10);
    const [params] = engine.callsTo('events.expand');
    expect(params.calendarIds).toEqual(['work']);
    expect(params.start).toBe(100);
    expect(params.end).toBe(200);
    expect(typeof params.tz).toBe('string');
    expect(seen[seen.length - 1].map((o) => o.calendarId)).toEqual(['work']);
  });
});

describe('occurrencesForEvents', function () {
  it('asks the engine for the rows by id', async function () {
    const engine = fakePimEngine();
    engine.answers['events.expand'] = () => ({ occurrences: [allDay('2026-06-22', '2026-06-22')] });
    const event = new EventModel({ id: 'e-1', accountId: 'a' } as any);
    const found = await occurrencesForEvents([event], { startUnix: 1, endUnix: 2 });
    expect(found.length).toBe(1);
    const [params] = engine.callsTo('events.expand');
    expect([params.eventIds, params.start, params.end]).toEqual([['e-1'], 1, 2]);
  });
});

describe('focusedEventInfoForEvents', function () {
  it('focuses the occurrence the engine picks, by id and start', async function () {
    const engine = fakePimEngine();
    engine.answers['events.expand'] = () => ({
      occurrences: [],
      focused: {
        ...allDay('2026-06-22', '2026-06-22'),
        id: 'e-1-e5',
        isAllDay: false,
        start: 5,
        end: 6,
      },
    });
    const event = new EventModel({ id: 'e-1', accountId: 'a' } as any);
    expect(await focusedEventInfoForEvents([event], 1000.5, 7)).toEqual({ id: 'e-1-e5', start: 5 });
    expect(engine.callsTo('events.expand')[0].focus).toEqual({ now: 1000, recurrenceIdStart: 7 });
  });

  it('has nothing to focus when the event is not on a calendar', async function () {
    expect(await focusedEventInfoForEvents([], 1000)).toBe(null);
  });
});
