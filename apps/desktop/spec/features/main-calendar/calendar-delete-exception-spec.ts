// Deleting from the grid asks the engine for the write (events.update, `deleted`); the ICS it
// writes (the override dropped, the slot excluded, SEQUENCE bumped) is crates/ms-pim's tests.
import { Actions, DatabaseStore, DestroyEventTask, SyncbackEventTask } from 'mailspring-exports';
import { dialog } from '@electron/remote';
import { Event as MailspringEvent } from '../../../src/data/models/event';
import { MailspringCalendar } from '../../../src/features/main-calendar/core/calendar-view';
import { TimedOccurrence } from '../../../src/features/main-calendar/core/calendar-data-source';
import { fakePimEngine, refusal } from '../../fake-pim-engine';

// A daily series whose 2 March occurrence has been moved to 08:00. Both rows the sync engine
// creates from this file share the one resource.
const MASTER_ICS = [
  'BEGIN:VCALENDAR',
  'VERSION:2.0',
  'PRODID:-//Test//Test//EN',
  'BEGIN:VEVENT',
  'UID:series@test',
  'DTSTART:20260301T060000Z',
  'DTEND:20260301T070000Z',
  'RRULE:FREQ=DAILY;COUNT=10',
  'SUMMARY:Daily Standup',
  'DTSTAMP:20260101T000000Z',
  'SEQUENCE:0',
  'END:VEVENT',
  'BEGIN:VEVENT',
  'UID:series@test',
  'RECURRENCE-ID:20260302T060000Z',
  'DTSTART:20260302T080000Z',
  'DTEND:20260302T090000Z',
  'SUMMARY:Daily Standup',
  'DTSTAMP:20260101T000000Z',
  'SEQUENCE:0',
  'END:VEVENT',
  'END:VCALENDAR',
].join('\r\n');

const MASTER_START = Date.UTC(2026, 2, 1, 6, 0, 0) / 1000;
const MOVED_START = Date.UTC(2026, 2, 2, 8, 0, 0) / 1000;
const MOVED_SLOT = Date.UTC(2026, 2, 2, 6, 0, 0) / 1000;

const master = () =>
  new MailspringEvent({
    id: 'master-row',
    accountId: 'a',
    calendarId: 'c',
    icsuid: 'series@test',
    recurrenceId: '',
    ics: MASTER_ICS,
    recurrenceStart: MASTER_START,
    recurrenceEnd: MASTER_START + 3600,
  } as any);

const exceptionRow = (recurrenceId: string) =>
  new MailspringEvent({
    id: 'exception-row',
    accountId: 'a',
    calendarId: 'c',
    icsuid: 'series@test',
    recurrenceId,
    ics: MASTER_ICS,
    recurrenceStart: MOVED_START,
    recurrenceEnd: MOVED_START + 3600,
  } as any);

const occurrence: TimedOccurrence = {
  id: 'exception-row-e0',
  accountId: 'a',
  calendarId: 'c',
  title: 'Daily Standup',
  location: '',
  description: '',
  isAllDay: false,
  isCancelled: false,
  isPending: false,
  isException: true,
  isRecurring: false,
  recurrenceIdStart: MOVED_SLOT,
  organizer: null,
  attendees: [],
  start: MOVED_START,
  end: MOVED_START + 3600,
} as TimedOccurrence;

let engine: ReturnType<typeof fakePimEngine>;
let queued: any[];

function stubEngine() {
  engine = fakePimEngine();
  engine.answers['events.update'] = (params) => ({
    writes: [{ kind: 'update', eventId: params.eventId, ics: MASTER_ICS }],
  });
  queued = [];
  spyOn(Actions, 'queueTask').andCallFake((task) => queued.push(task));
  spyOn(AppEnv, 'showErrorDialog');
}

const update = () => engine.callsTo('events.update')[0];

describe('deleting a moved occurrence from the calendar', function () {
  let calendar: any;

  function stubRows(exception: MailspringEvent, masterRow: MailspringEvent | null) {
    spyOn(DatabaseStore, 'find').andReturn(Promise.resolve(exception));
    spyOn(DatabaseStore, 'findBy').andReturn(Promise.resolve(masterRow));
  }

  beforeEach(function () {
    stubEngine();
    // _deleteEvent reads nothing from the rendered tree, so the component need not be mounted.
    calendar = new MailspringCalendar({} as any);
  });

  it('edits the master instead of destroying the resource the rows share', async function () {
    stubRows(exceptionRow('20260302T060000Z'), master());

    await calendar._deleteEvent(occurrence);

    expect(update().eventId).toBe('master-row');
    expect(update().scope).toBe('this');
    expect(update().occurrenceStart).toBe(MOVED_SLOT);
    expect(update().patch).toEqual({ deleted: true });
    expect(queued.length).toBe(1);
    expect(queued[0] instanceof SyncbackEventTask).toBe(true);
    expect(queued[0].event.id).toBe('master-row');
    expect(queued.some((t) => t instanceof DestroyEventTask)).toBe(false);
  });

  it('lets the engine read the slot off the row when the occurrence does not say it', async function () {
    stubRows(exceptionRow('20260302T060000Z'), master());

    await calendar._deleteEvent({ ...occurrence, recurrenceIdStart: undefined });

    expect(update().eventId).toBe('exception-row');
    expect(update().scope).toBe('this');
    expect(update().occurrenceStart).toBeUndefined();
  });

  it('reports a delete the engine refuses, rather than deleting', async function () {
    engine.answers['events.update'] = () => {
      throw refusal('invalid-params');
    };
    stubRows(exceptionRow('20991231T060000Z'), master());

    await calendar._deleteEvent(occurrence);

    expect(queued.length).toBe(0);
    expect(AppEnv.showErrorDialog).toHaveBeenCalled();
  });
});

describe('deleting one plain occurrence from the calendar', function () {
  const start = MASTER_START + 2 * 86400;
  const plain = {
    ...occurrence,
    id: 'master-row-e2',
    isException: false,
    isRecurring: true,
    recurrenceIdStart: undefined,
    start,
    end: start + 3600,
  };
  let choice: jasmine.Spy;

  beforeEach(function () {
    stubEngine();
    spyOn(DatabaseStore, 'find').andReturn(Promise.resolve(master()));
    // The recurring-event dialog's first button is "this occurrence".
    choice = spyOn(dialog, 'showMessageBoxSync').andReturn(0);
  });

  it('removes that slot alone when told to', async function () {
    const calendar: any = new MailspringCalendar({} as any);

    await calendar._deleteEvent(plain);

    expect(update().eventId).toBe('master-row');
    expect(update().scope).toBe('this');
    expect(update().occurrenceStart).toBe(start);
    expect(update().patch).toEqual({ deleted: true });
    expect(queued.length).toBe(1);
  });

  it('destroys the series when told to delete every occurrence', async function () {
    choice.andReturn(1);
    const calendar: any = new MailspringCalendar({} as any);

    await calendar._deleteEvent(plain);

    expect(engine.calls.length).toBe(0);
    expect(queued.length).toBe(1);
    expect(queued[0] instanceof DestroyEventTask).toBe(true);
  });
});
