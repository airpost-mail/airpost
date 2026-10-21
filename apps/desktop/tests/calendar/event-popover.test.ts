// The popover's save path: what it asks the engine to change (events.update with queue: false)
// and the undoable task it queues. The ICS the engine writes for these edits (a detailed RRULE
// or RDATEs kept, EXDATEs rezoned, one SEQUENCE bump, ORGANIZER on the first guest, TRANSP per
// VEVENT) is crates/tern-pim's desktop corpus, recorded from this spec.
import * as Actions from '../../src/data/actions';
import DatabaseStore from '../../src/data/database/store';
import { SyncbackEventTask } from '../../src/data/tasks/syncback-event';
import { Event as EventModel } from '../../src/data/models/event';
import { CalendarEventPopover } from '../../src/features/main-calendar/event/popover';
import { TimedOccurrence } from '../../src/features/main-calendar/data-source';
import { AllDayToggle } from '../../src/features/main-calendar/event/all-day-toggle';
import { ShowAsSelector } from '../../src/features/main-calendar/event/show-as-selector';
import { fakePimEngine, flush } from '../support/fake-pim-engine';

// A fortnightly Tue/Thu meeting that ends in June. Every part of this rule beyond FREQ is
// something the five-value Repeat control cannot express.
const DETAILED_RRULE = 'FREQ=WEEKLY;INTERVAL=2;BYDAY=TU,TH;UNTIL=20260630T000000Z';
const ICS =
  'BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:planning@test\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n';
const WRITTEN = ICS.replace('UID', 'SEQUENCE:1\r\nUID');

const START = Date.UTC(2026, 2, 3, 14, 0, 0) / 1000;
const END = Date.UTC(2026, 2, 3, 15, 0, 0) / 1000;

function makeEvent() {
  return new EventModel({
    id: 'event-1',
    accountId: 'account-1',
    calendarId: 'calendar-1',
    ics: ICS,
    recurrenceStart: START,
    recurrenceEnd: END,
  } as any);
}

// What the calendar's data source gives the popover, from events.expand: the series' rule and
// zone come with the occurrence.
function makeOccurrence(title: string, extra: Partial<TimedOccurrence> = {}): TimedOccurrence {
  return {
    id: 'event-1-e0',
    eventId: 'event-1',
    accountId: 'account-1',
    calendarId: 'calendar-1',
    title,
    location: '',
    description: '',
    isAllDay: false,
    isCancelled: false,
    isPending: false,
    isMine: true,
    isException: false,
    isRecurring: true,
    organizer: null,
    attendees: [],
    start: START,
    end: END,
    rrule: DETAILED_RRULE,
    timezone: null,
    ...extra,
  } as TimedOccurrence;
}

// The editor is exercised the way the UI drives it: onEdit reads the event's recurrence into
// state, updateField is what every input calls, and _saveAllOccurrences is the save. The
// component is not mounted, so setState is redirected straight into state.
async function openEditor(title: string, extra: Partial<TimedOccurrence> = {}) {
  const popover: any = new CalendarEventPopover({
    event: makeOccurrence(title, extra),
    onEdit: () => {},
    onDelete: () => {},
  } as any);
  popover.setState = (update: object) => Object.assign(popover.state, update);
  await popover.onEdit();
  return popover;
}

let engine: ReturnType<typeof fakePimEngine>;
let queued: any[];

function stubEngine() {
  engine = fakePimEngine();
  engine.answers['events.update'] = (params) => ({
    writes: [{ kind: 'update', eventId: params.eventId, ics: WRITTEN }],
  });
  queued = [];
  spyOn(Actions, 'queueTask').andCallFake((task) => queued.push(task));
}

const updates = () => engine.callsTo('events.update');
const update = () => updates()[0];

describe('CalendarEventPopover save path and the recurrence rule', function () {
  beforeEach(stubEngine);

  it('leaves a detailed RRULE to the engine when only the title changed', async function () {
    const popover = await openEditor('Planning');
    expect(popover.state.repeat).toBe('weekly');
    popover.updateField('title', 'Planning (renamed)');

    await popover._saveAllOccurrences(makeEvent(), true);

    expect(update().scope).toBe('all');
    expect(update().occurrenceStart).toBe(START);
    expect(update().patch.summary).toBe('Planning (renamed)');
    expect('recurrenceRule' in update().patch).toBe(false);
    expect('timezone' in update().patch).toBe(false);
  });

  it('reads an RDATE-only series as not repeating, and sends no rule for it', async function () {
    const popover = await openEditor('Board meeting', { rrule: null });
    expect(popover.state.repeat).toBe('none');
    popover.updateField('title', 'Board meeting (renamed)');

    await popover._saveAllOccurrences(makeEvent(), true);

    expect('recurrenceRule' in update().patch).toBe(false);
  });

  it('sends the zone when the zone picker changed', async function () {
    const popover = await openEditor('Standup', { timezone: 'Europe/Berlin' });
    expect(popover.state.timezone).toBe('Europe/Berlin');
    popover.updateField('timezone', 'America/Chicago');

    await popover._saveAllOccurrences(makeEvent(), true);

    expect(update().patch.timezone).toBe('America/Chicago');
  });

  it('always sends the zone of an event that does not repeat', async function () {
    const popover = await openEditor('Lunch', { rrule: null, isRecurring: false });
    await popover._saveAllOccurrences(makeEvent(), false);
    expect(update().patch.timezone).toBe(popover.state.timezone);
    expect(update().occurrenceStart).toBeUndefined();
  });

  it('writes the rule the user picked when the Repeat control changed', async function () {
    const popover = await openEditor('Planning');
    popover.updateField('repeat', 'daily');

    await popover._saveAllOccurrences(makeEvent(), true);

    expect(update().patch.recurrenceRule).toBe('FREQ=DAILY');
  });
});

describe('CalendarEventPopover save path and the engine', function () {
  beforeEach(stubEngine);

  it('sends one save as one change, queued as one undoable task', async function () {
    const event = makeEvent();
    const popover = await openEditor('Planning');
    popover.updateField('title', 'Planning (renamed)');
    popover.updateField('repeat', 'daily');
    popover.updateStart(START + 3600);

    await popover._saveAllOccurrences(event, true);

    expect(updates().length).toBe(1);
    expect(update().queue).toBe(false);
    expect(update().patch.start).toBe(START + 3600);
    expect(queued.length).toBe(1);
    expect(queued[0] instanceof SyncbackEventTask).toBe(true);
    expect(queued[0].event.ics).toBe(WRITTEN);
    expect(queued[0].undoData.ics).toBe(ICS);
  });

  it('edits one occurrence in the slot it fills', async function () {
    const popover = await openEditor('Planning');
    popover.updateStart(START + 3600);

    await popover._saveOccurrenceException(makeEvent());

    expect(update().scope).toBe('this');
    expect(update().occurrenceStart).toBe(START);
    expect(update().patch.start).toBe(START + 3600);
  });

  it('edits an occurrence already moved in the slot it fills, not where it is now', async function () {
    const popover = await openEditor('Planning', {
      isException: true,
      start: START + 7200,
      end: END + 7200,
      recurrenceIdStart: START,
    } as any);

    await popover._saveOccurrenceException(makeEvent());

    expect(update().occurrenceStart).toBe(START);
  });

  it('sends the guests, for the engine to name the account as organizer', async function () {
    const popover = await openEditor('Planning');
    popover.updateAttendees([{ email: 'bo@example.com', name: 'Bo' }]);

    await popover._saveAllOccurrences(makeEvent(), true);

    expect(update().patch.attendees).toEqual([{ email: 'bo@example.com', name: 'Bo' }]);
  });
});

describe('CalendarEventPopover and who may edit', function () {
  const popoverFor = (props: object) =>
    new CalendarEventPopover({
      event: makeOccurrence('Planning'),
      onEdit: () => {},
      onDelete: () => {},
      ...props,
    } as any) as any;

  it('does not offer to edit a meeting somebody else organizes', function () {
    const theirs = { ...makeOccurrence('Planning'), isMine: false };
    expect(popoverFor({ event: theirs })._isEditable()).toBe(false);
  });

  it('offers to edit a meeting we organize on a writable calendar', function () {
    expect(popoverFor({})._isEditable()).toBe(true);
  });

  it('never offers to edit on a read-only calendar', function () {
    expect(popoverFor({ isCalendarReadOnly: true })._isEditable()).toBe(false);
  });

  it('always lets a new event be edited', function () {
    const theirs = { ...makeOccurrence('Planning'), isMine: false };
    expect(popoverFor({ event: theirs, isNewEvent: true })._isEditable()).toBe(true);
  });

  it('opens straight into the editor when asked, reading the rule off the occurrence first', async function () {
    const popover = popoverFor({ startEditing: true });
    popover.setState = (update: object) => Object.assign(popover.state, update);
    expect(popover.state.editing).toBe(true);
    expect(popover.state.repeat).toBe('none');

    popover.componentDidMount();
    await flush();

    expect(popover.state.repeat).toBe('weekly');
    expect(popover.state.originalRepeat).toBe('weekly');
  });

  it('does not read a rule for a new event, which has none yet', async function () {
    const find = spyOn(DatabaseStore, 'find');
    const popover = popoverFor({ startEditing: true, isNewEvent: true });
    popover.componentDidMount();
    expect(find).not.toHaveBeenCalled();
  });
});

describe('CalendarEventPopover save path and Show As', function () {
  beforeEach(stubEngine);

  it('shows a free event as Free', async function () {
    const free = await openEditor('Planning', { isFree: true });
    expect(free.state.showAs).toBe('TRANSPARENT');
  });

  it('shows a busy event as Busy', async function () {
    const busy = await openEditor('Planning', { isFree: false });
    expect(busy.state.showAs).toBe('OPAQUE');
  });

  it('marks the series free when Show As changes to Free', async function () {
    const popover = await openEditor('Planning');
    popover.updateShowAs('TRANSPARENT');

    await popover._saveAllOccurrences(makeEvent(), true);

    expect(update().scope).toBe('all');
    expect(update().patch.transparency).toBe('TRANSPARENT');
  });

  it('marks a free series busy when Show As changes to Busy', async function () {
    const popover = await openEditor('Planning', { isFree: true });
    popover.updateShowAs('OPAQUE');

    await popover._saveAllOccurrences(makeEvent(), true);

    expect(update().patch.transparency).toBe('OPAQUE');
  });

  it('leaves TRANSP as it was when Show As was not touched', async function () {
    const popover = await openEditor('Planning');
    popover.updateField('title', 'Planning (renamed)');

    await popover._saveAllOccurrences(makeEvent(), true);

    expect(update().patch.transparency).toBeUndefined();
  });

  it('marks only the occurrence free when one occurrence is edited', async function () {
    const popover = await openEditor('Planning');
    popover.updateShowAs('TRANSPARENT');

    await popover._saveOccurrenceException(makeEvent());

    expect(update().scope).toBe('this');
    expect(update().patch.transparency).toBe('TRANSPARENT');
  });

  it('adds no TRANSP to an occurrence whose Show As was not touched', async function () {
    const popover = await openEditor('Planning');
    popover.updateField('title', 'Planning (moved)');

    await popover._saveOccurrenceException(makeEvent());

    expect(update().patch.transparency).toBeUndefined();
  });

  it('follows the event when it changes under the editor', async function () {
    const popover = await openEditor('Planning');
    const prevProps = popover.props;
    popover.props = { ...prevProps, event: makeOccurrence('Planning', { isFree: true }) };
    popover.componentDidUpdate(prevProps, popover.state);
    expect(popover.state.showAs).toBe('TRANSPARENT');
    expect(popover._changedShowAs()).toBe(undefined);
  });

  // A popover for an event that does not exist yet, as the grid opens one.
  const newEventPopover = (isAllDay: boolean) => {
    const popover: any = new CalendarEventPopover({
      event: makeOccurrence('', { isAllDay } as any),
      isNewEvent: true,
    } as any);
    popover.setState = (update: object) => Object.assign(popover.state, update);
    return popover;
  };

  /** The first element of this type in a rendered tree. */
  const findElement = (node: any, type: any): any => {
    if (!node || typeof node !== 'object') return null;
    if (Array.isArray(node)) {
      for (const child of node) {
        const found = findElement(child, type);
        if (found) return found;
      }
      return null;
    }
    if (node.type === type) return node;
    return findElement(node.props && node.props.children, type);
  };

  it('opens a new all-day event as Free and a new timed one as Busy', function () {
    expect(newEventPopover(true).state.showAs).toBe('TRANSPARENT');
    expect(newEventPopover(false).state.showAs).toBe('OPAQUE');
  });

  it('opens an existing all-day event with the Show As it has', async function () {
    const busy = await openEditor('Planning', {
      isAllDay: true,
      isFree: false,
    } as any);
    expect(busy.state.showAs).toBe('OPAQUE');
  });

  it('moves a new event between Busy and Free with the all-day switch', function () {
    const popover = newEventPopover(false);
    popover.updateAllDay(true);
    expect(popover.state.allDay).toBe(true);
    expect(popover.state.showAs).toBe('TRANSPARENT');
    popover.updateAllDay(false);
    expect(popover.state.showAs).toBe('OPAQUE');
  });

  it("keeps a new event's Show As once the user has picked one", function () {
    const popover = newEventPopover(false);
    popover.updateShowAs('OPAQUE');
    popover.updateAllDay(true);
    expect(popover.state.allDay).toBe(true);
    expect(popover.state.showAs).toBe('OPAQUE');
  });

  it("leaves an existing event's Show As alone when it is made all-day", async function () {
    const popover = await openEditor('Planning');
    popover.updateAllDay(true);
    expect(popover.state.allDay).toBe(true);
    expect(popover.state.showAs).toBe('OPAQUE');
  });

  it('follows the all-day switch again for the next event the editor is given', function () {
    const popover = newEventPopover(false);
    popover.updateShowAs('OPAQUE');
    const prevProps = popover.props;
    popover.props = { ...prevProps, event: makeOccurrence('Other', { isAllDay: false } as any) };
    popover.componentDidUpdate(prevProps, popover.state);
    popover.updateAllDay(true);
    expect(popover.state.showAs).toBe('TRANSPARENT');
  });

  it('wires the all-day switch and Show As control to their handlers', function () {
    const popover = newEventPopover(false);
    const tree = popover.renderEditable();
    expect(findElement(tree, AllDayToggle).props.onChange).toBe(popover.updateAllDay);
    expect(findElement(tree, ShowAsSelector).props.onChange).toBe(popover.updateShowAs);
  });

  it('creates a new all-day event as Free when Show As was left alone', async function () {
    const helpers = require('../../src/features/main-calendar/helpers');
    spyOn(helpers, 'createCalendarEvent').andReturn(Promise.resolve());
    spyOn(Actions, 'closePopover');
    await newEventPopover(true)._createNewEvent();
    expect(helpers.createCalendarEvent.mostRecentCall.args[0].transparency).toBe('TRANSPARENT');
  });

  it('creates the event with the Show As the user picked', async function () {
    const helpers = require('../../src/features/main-calendar/helpers');
    spyOn(helpers, 'createCalendarEvent').andReturn(Promise.resolve());
    spyOn(Actions, 'closePopover');
    const popover: any = new CalendarEventPopover({
      event: makeOccurrence(''),
      isNewEvent: true,
    } as any);
    popover.setState = (update: object) => Object.assign(popover.state, update);
    popover.updateShowAs('TRANSPARENT');

    await popover._createNewEvent();

    expect(helpers.createCalendarEvent.mostRecentCall.args[0].transparency).toBe('TRANSPARENT');
  });
});
