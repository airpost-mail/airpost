import Rx from 'rx-lite';
import { Event } from '../../data/models/event';
import DatabaseStore from '../../data/database/store';
import { CalendarDate } from '../../data/calendar/calendar-date';
import * as CalendarDateUtils from '../../data/calendar/calendar-date';
import { Calendar } from '../../data/models/calendar';
import EngineEvents from '../../engine/client/events';
import { expandEvents } from '../../engine/client/pim';

/** Participation status values from iCalendar spec */
export type ParticipationStatus = 'NEEDS-ACTION' | 'ACCEPTED' | 'DECLINED' | 'TENTATIVE' | string;

/**
 * Represents an attendee of a calendar event with basic contact info.
 * Compatible with Contact model for use with EventAttendeesInput.
 */
export interface EventAttendee {
  email: string;
  name?: string | null;
  partstat?: ParticipationStatus;
}

/** Whether an occurrence covers the given day. */
export function eventCoversDate(event: EventOccurrence, date: CalendarDate): boolean {
  return event.startDate <= date && date <= event.endDate;
}

/**
 * Whether an occurrence is in the selection, compared by id. Selection holds occurrences captured
 * at click time; a data refresh gives every occurrence a fresh object identity, so an identity
 * (`includes`) check silently drops the highlight on the next render.
 */
export function isEventSelected(
  selectedEvents: EventOccurrence[],
  event: EventOccurrence
): boolean {
  return selectedEvents.some((e) => e.id === event.id);
}

/**
 * Narrowing guard. tests/ are type-checked without `strictNullChecks`, where `if (e.isAllDay)`
 * does NOT narrow the union — only `=== true`/`=== false` and a guard like this do. Use it to
 * reach `start`/`end`, which exist on timed occurrences only.
 */
export function isTimed(e: EventOccurrence): e is TimedOccurrence {
  return e.isAllDay === false;
}

/**
 * An occurrence's start as an instant — its real start if timed, its first day's start if
 * all-day. For sort keys, the focus scroll target, and day-snap drag references; never stored.
 */
export function occurrenceStartUnix(e: EventOccurrence): number {
  return isTimed(e) ? e.start : CalendarDateUtils.dayStartUnix(e.startDate);
}

/** The id an occurrence expanded from `eventId`'s ICS is drawn under; a move gives it a new one. */
export function occurrenceId(eventId: string, start: number): string {
  return `${eventId}-e${Math.round(start)}`;
}

/**
 * An occurrence's end as an exclusive instant — its real end if timed, the midnight after its
 * last covered day if all-day. For drag hit-testing and time formatting; never stored, and not
 * for layout positioning (that stays in date space).
 */
export function occurrenceEndUnix(e: EventOccurrence): number {
  return isTimed(e) ? e.end : CalendarDateUtils.nextDayStartUnix(e.endDate);
}

/**
 * The dates an event covers, inclusive, from its start and exclusive end instants.
 *
 * All-day ends land on a day boundary, so the last covered date is one date back; timed ends
 * are instants, so step inside the end before reading it — a 22:00-00:00 event covers one day,
 * not two. For events with parsed ICS, read the DATE parts instead; this is for callers that
 * only hold instants.
 */
export function coveredDates(
  startUnix: number,
  endUnix: number,
  isAllDay: boolean
): { startDate: CalendarDate; endDate: CalendarDate } {
  const startDate = CalendarDateUtils.calendarDateFromUnix(startUnix);
  const endDate = isAllDay
    ? CalendarDateUtils.addCalendarDays(CalendarDateUtils.calendarDateFromUnix(endUnix), -1)
    : CalendarDateUtils.calendarDateFromUnix(Math.max(endUnix - 1, startUnix));
  return { startDate, endDate: endDate > startDate ? endDate : startDate };
}

/** Fields common to all occurrences, all-day or timed. */
interface OccurrenceBase {
  /**
   * The days covered, inclusive both ends — a one-day event has `startDate === endDate`.
   *
   * The authoritative span for all-day events, and what month rows (layoutWeekBars) and agenda
   * days (eventCoversDate) place events by.
   */
  startDate: CalendarDate;
  endDate: CalendarDate;
  id: string;
  accountId: string;
  calendarId: string;
  title: string;
  location: string;
  description: string;
  isCancelled: boolean;
  /**
   * True if this event should display with "pending" styling (hatched pattern).
   * This includes:
   * - Events with STATUS=TENTATIVE
   * - Events where the current user is an attendee but hasn't accepted (NEEDS-ACTION or TENTATIVE)
   */
  isPending: boolean;
  /**
   * True when this account organizes the event, or nobody does, or its ORGANIZER is the address
   * of the calendar it sits on and the server says that calendar is ours (Google writes a
   * secondary calendar's own id as ORGANIZER). Governs who may reschedule it.
   */
  isMine: boolean;
  /** TRANSP:TRANSPARENT on this occurrence's own VEVENT: it does not block time. */
  isFree?: boolean;
  isException: boolean;
  /**
   * For exception occurrences only: the Unix timestamp (seconds) of the **original**
   * unmodified occurrence start, taken from the RECURRENCE-ID property. This differs
   * from `start` when the exception has been moved to a different time.
   * Used by the edit popover to correctly upsert the existing inline exception VEVENT.
   */
  recurrenceIdStart?: number;
  /** True if this event is part of a recurring series (has RRULE/RDATE) */
  isRecurring: boolean;
  organizer: { email: string } | null;
  attendees: EventAttendee[];
  /** The Event row it was expanded from, and the row's UID. */
  eventId?: string;
  icsuid?: string;
  /** This account's PARTSTAT on the guest list, upper-cased; null when it isn't a guest. */
  myStatus?: string | null;
  /** The series' RRULE value, and the zone its times are written in. */
  rrule?: string | null;
  timezone?: string | null;

  /** True if this is a synthetic drag preview event (not a real event) */
  isDragPreview?: boolean;
  /** If this is a preview, the ID of the original event being dragged */
  originalEventId?: string;
}

/** An all-day occurrence: covered dates only, no instants. `isAllDay` discriminates the union. */
export interface AllDayOccurrence extends OccurrenceBase {
  isAllDay: true;
}

/** A timed occurrence: keeps its start/end instants. */
export interface TimedOccurrence extends OccurrenceBase {
  isAllDay: false;
  start: number; // unix
  end: number; // unix
}

export type EventOccurrence = AllDayOccurrence | TimedOccurrence;

// Minimal type for focusing/highlighting an event on the calendar
/**
 * What the calendar keeps to hold an event focused: its id and a scroll-to instant. `start`
 * is that instant — a timed event's real start, an all-day event's day start — so an all-day
 * occurrence (which has no start of its own) still scrolls the view somewhere sensible.
 */
export type FocusedEventInfo = { id: string; start: number };

/**
 * The occurrences the engine expands (`events.expand`) for the shown range, on the calendars
 * not switched off, again whenever an Event or Calendar row changes or the engine connection
 * comes back. A request that fails (the engine restarting) keeps what is shown.
 */
export class CalendarDataSource {
  // Set by buildObservable(), which callers run before subscribe().
  observable!: Rx.Observable<{ events: EventOccurrence[] }>;

  buildObservable({
    startUnix,
    endUnix,
    disabledCalendars,
  }: {
    startUnix: number;
    endUnix: number;
    disabledCalendars?: string[];
  }) {
    const disabled = new Set(disabledCalendars || []);
    const calendars = Rx.Observable.fromQuery(DatabaseStore.findAll<Calendar>(Calendar));
    const eventChanges = Rx.Observable.create<null>((observer) => {
      observer.onNext(null);
      const unlisten = DatabaseStore.listen((record: { objectClass: string }) => {
        if (record.objectClass === Event.name) observer.onNext(null);
      });
      const onConnected = () => observer.onNext(null);
      EngineEvents.on('connected', onConnected);
      return Rx.Disposable.create(() => {
        unlisten();
        EngineEvents.removeListener('connected', onConnected);
      });
    });
    this.observable = Rx.Observable.combineLatest(
      calendars,
      eventChanges,
      (cals: Calendar[]) => cals
    )
      .flatMapLatest((cals: Calendar[]): Rx.Observable<{ events: EventOccurrence[] } | null> => {
        const calendarIds = cals.filter((c) => !disabled.has(c.id)).map((c) => c.id);
        if (!calendarIds.length) {
          return Rx.Observable.just<{ events: EventOccurrence[] } | null>({ events: [] });
        }
        return Rx.Observable.fromPromise<{ events: EventOccurrence[] } | null>(
          expandEvents({ calendarIds, start: startUnix, end: endUnix })
            .then((r) => ({ events: (r.occurrences || []) as EventOccurrence[] }))
            .catch((err) => {
              console.warn(`Calendar: could not expand events: ${err.message}`);
              return null;
            })
        );
      })
      // rx-lite's filter() has no type-guard overload; this drops the nulls.
      .filter((result) => result !== null) as Rx.Observable<{ events: EventOccurrence[] }>;
    return this.observable;
  }

  subscribe(callback: (result: { events: EventOccurrence[] }) => void) {
    return this.observable.subscribe(callback);
  }
}

/**
 * The occurrences of `events` (with the rest of each one's series) that overlap the range, as
 * the engine expands them.
 */
export async function occurrencesForEvents(
  events: Event[],
  { startUnix, endUnix }: { startUnix: number; endUnix: number }
): Promise<EventOccurrence[]> {
  if (!events.length) return [];
  const result = await expandEvents({
    eventIds: events.map((e) => e.id),
    start: startUnix,
    end: endUnix,
  });
  return (result.occurrences || []) as EventOccurrence[];
}

/**
 * The occurrence to focus for an event opened from outside the calendar: the one
 * `recurrenceIdStart` names when the request is about a single occurrence of a series,
 * otherwise the next one from `nowUnix`, or the event's first when none is upcoming.
 *
 * @param events - Every Event row sharing one UID, so a series' exceptions are placed.
 */
export async function focusedEventInfoForEvents(
  events: Event[],
  nowUnix: number,
  recurrenceIdStart?: number
): Promise<FocusedEventInfo | null> {
  if (!events.length) return null;
  const now = Math.floor(nowUnix);
  const result = await expandEvents({
    eventIds: [events[0].id],
    start: now,
    end: now,
    focus: { now, recurrenceIdStart },
  });
  const occurrence = result.focused as EventOccurrence | null;
  return occurrence ? { id: occurrence.id, start: occurrenceStartUnix(occurrence) } : null;
}
