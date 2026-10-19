// Moves and resizes from the calendar grid go to the engine (events.update with queue: false),
// which builds the ICS; the window queues the writes itself so they can be undone. The ICS edits
// are crates/ms-pim's tests.
import { Actions, SyncbackEventTask } from 'mailspring-exports';
import { Event as MailspringEvent } from '../src/flux/models/event';
import {
  modifySimpleEvent,
  createOccurrenceException,
  modifyAllOccurrences,
  modifyEventWithRecurringSupport,
  EventTimeChangeOptions,
} from '../internal_packages/main-calendar/lib/core/recurring-event-actions';
import { fakePimEngine } from './fake-pim-engine';

const OLD_ICS =
  'BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u@test\r\nSEQUENCE:0\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n';
const NEW_ICS = OLD_ICS.replace('SEQUENCE:0', 'SEQUENCE:1');

const T_OCC2_START = Date.UTC(2026, 2, 2, 6) / 1000;
const T_NEW_START = Date.UTC(2026, 2, 2, 8) / 1000;
const T_NEW_END = Date.UTC(2026, 2, 2, 9) / 1000;

function makeEvent(overrides: Partial<MailspringEvent> = {}): MailspringEvent {
  return new MailspringEvent({
    id: 'event-id-1',
    accountId: 'account-id-1',
    calendarId: 'calendar-id-1',
    icsuid: 'u@test',
    ics: OLD_ICS,
    recurrenceStart: Date.UTC(2026, 2, 1, 6) / 1000,
    recurrenceEnd: Date.UTC(2026, 2, 1, 7) / 1000,
    ...overrides,
  } as any);
}

function options(over: Partial<EventTimeChangeOptions> = {}): EventTimeChangeOptions {
  return {
    event: makeEvent(),
    originalOccurrenceStart: T_OCC2_START,
    newStart: T_NEW_START,
    newEnd: T_NEW_END,
    isAllDay: false,
    ...over,
  };
}

describe('recurring event actions', function () {
  let engine: ReturnType<typeof fakePimEngine>;
  let queued: SyncbackEventTask[];
  let dialog: jasmine.Spy;

  beforeEach(function () {
    engine = fakePimEngine();
    engine.answers['events.update'] = (params) => ({
      writes: [{ kind: 'update', eventId: params.eventId, ics: NEW_ICS }],
    });
    queued = [];
    spyOn(Actions, 'queueTask').andCallFake((t) => queued.push(t));
    dialog = spyOn(require('@electron/remote').dialog, 'showMessageBoxSync').andReturn(0);
  });

  const update = () => engine.callsTo('events.update')[0];

  describe('modifySimpleEvent', function () {
    it('moves the whole event, without queueing it on the engine', async function () {
      await modifySimpleEvent(options());
      expect(update().scope).toBe('all');
      expect(update().patch).toEqual({ start: T_NEW_START, end: T_NEW_END, isAllDay: false });
      expect(update().queue).toBe(false);
      expect(update().eventId).toBe('event-id-1');
      expect(typeof update().tz).toBe('string');
    });

    it('queues the write as an undoable SyncbackEventTask', async function () {
      await modifySimpleEvent(options({ description: 'Move event' }));
      expect(queued.length).toBe(1);
      const task = queued[0];
      expect(task instanceof SyncbackEventTask).toBe(true);
      expect(task.event.ics).toBe(NEW_ICS);
      expect(task.undoData.ics).toBe(OLD_ICS);
      expect(task.canBeUndone).toBe(true);
      expect(task.description()).toBe('Move event');
    });

    it('edits the override a standalone exception row names', async function () {
      await modifySimpleEvent(options({ event: makeEvent({ recurrenceId: '20260302T060000Z' }) }));
      expect(update().scope).toBe('this');
      expect(update().occurrenceStart).toBeUndefined();
    });
  });

  describe('createOccurrenceException', function () {
    it('moves the occurrence in the slot it fills', async function () {
      await createOccurrenceException(options());
      expect(update().scope).toBe('this');
      expect(update().occurrenceStart).toBe(T_OCC2_START);
      expect(queued[0].description()).toBe('Edit occurrence');
    });
  });

  describe('modifyAllOccurrences', function () {
    it('moves the series by the change made to the occurrence', async function () {
      await modifyAllOccurrences(options());
      expect(update().scope).toBe('all');
      expect(update().occurrenceStart).toBe(T_OCC2_START);
      expect(update().patch.start).toBe(T_NEW_START);
    });
  });

  describe('modifyEventWithRecurringSupport', function () {
    it('asks about a series and edits the one occurrence when told to', async function () {
      dialog.andReturn(0);
      const result = await modifyEventWithRecurringSupport(
        options({ isRecurring: true }),
        'move',
        'Daily Standup'
      );
      expect(dialog.callCount).toBe(1);
      expect(result.success).toBe(true);
      expect(update().scope).toBe('this');
    });

    it('edits every occurrence when told to', async function () {
      dialog.andReturn(1);
      await modifyEventWithRecurringSupport(options({ isRecurring: true }), 'move', 'Daily Standup');
      expect(update().scope).toBe('all');
      expect(update().occurrenceStart).toBe(T_OCC2_START);
    });

    it('does nothing when the question is dismissed', async function () {
      dialog.andReturn(2);
      const result = await modifyEventWithRecurringSupport(
        options({ isRecurring: true }),
        'move',
        'Daily Standup'
      );
      expect(result).toEqual({ success: false, cancelled: true });
      expect(engine.calls.length).toBe(0);
    });

    it('edits an inline exception alone without asking', async function () {
      await modifyEventWithRecurringSupport(
        options({ isRecurring: true, isException: true }),
        'move',
        'Daily Standup'
      );
      expect(dialog.callCount).toBe(0);
      expect(update().scope).toBe('this');
      expect(update().occurrenceStart).toBe(T_OCC2_START);
    });

    it('moves an event that does not repeat without asking', async function () {
      await modifyEventWithRecurringSupport(options({ isRecurring: false }), 'move', 'Lunch');
      expect(dialog.callCount).toBe(0);
      expect(update().scope).toBe('all');
    });

    it('moves a standalone exception row without asking', async function () {
      await modifyEventWithRecurringSupport(
        options({
          event: makeEvent({ recurrenceId: '20260302T060000Z' }),
          isRecurring: true,
          isException: true,
        }),
        'move',
        'Daily Standup'
      );
      expect(dialog.callCount).toBe(0);
      expect(update().scope).toBe('this');
    });
  });
});
