import { Calendar } from '../../data/models/calendar';
import * as CalendarUtils from '../../data/calendar/ics';
import { Event } from '../../data/models/event';
import * as Utils from '../../data/models/utils';
import ICAL from 'ical.js';

/** Where an RSVP will be recorded, once we're certain which copy of the event is ours. */
export interface RSVPTarget {
  event: Event;
  calendar: Calendar;
}

/** Why no copy can be answered. The reply still goes to the organizer by email. */
export type RSVPTargetProblem =
  /** The event isn't on any calendar we sync. */
  | 'not-on-a-calendar'
  /** Every copy we found is on a read-only calendar. */
  | 'read-only'
  /** Every writable copy is on a calendar the server says belongs to somebody else. */
  | 'not-ours'
  /** Copies on more than one writable calendar, and none is identifiably ours. */
  | 'ambiguous';

export type RSVPTargetResolution =
  | { target: RSVPTarget; problem?: undefined }
  | { target: null; problem: RSVPTargetProblem };

/**
 * Picks the copy of an invitation this account may answer. One UID lands on our calendar, a
 * room's and a colleague's at once, so a target is named only when it is certainly ours.
 */
export function resolveRSVPTarget({
  events,
  calendars,
  addresses,
}: {
  events: Event[];
  calendars: Calendar[];
  addresses: string[];
}): RSVPTargetResolution {
  if (!events.length) {
    return { target: null, problem: 'not-on-a-calendar' };
  }

  const calendarsById = new Map(calendars.map((c) => [c.id, c]));
  const writable = events.filter((e) => {
    const calendar = calendarsById.get(e.calendarId);
    return calendar && !calendar.readOnly;
  });
  if (!writable.length) {
    return { target: null, problem: 'read-only' };
  }

  // The sole-candidate fallback below is for calendars of unknown ownership, never for one
  // the server named as someone else's.
  const calendarIds = new Set(
    writable
      .filter((e) => !CalendarUtils.isSomeoneElsesCalendar(calendarsById.get(e.calendarId)))
      .map((e) => e.calendarId)
  );
  if (!calendarIds.size) {
    return { target: null, problem: 'not-ours' };
  }
  const ownCalendarIds = [...calendarIds].filter((id) =>
    CalendarUtils.isOwnCalendar(calendarsById.get(id), addresses)
  );

  // Our own calendar, else the only candidate; never a guess between several.
  let calendarId: string;
  if (ownCalendarIds.length === 1) {
    calendarId = ownCalendarIds[0];
  } else if (ownCalendarIds.length === 0 && calendarIds.size === 1) {
    calendarId = [...calendarIds][0];
  } else {
    return { target: null, problem: 'ambiguous' };
  }

  // The master carries the attendee list for every occurrence; an exception answers only itself.
  const onCalendar = writable.filter((e) => e.calendarId === calendarId);
  const event = onCalendar.find((e) => !e.isRecurrenceException());
  if (!event) {
    return { target: null, problem: 'not-on-a-calendar' };
  }

  return { target: { event, calendar: calendarsById.get(calendarId) } };
}

/**
 * Whether an emailed invitation may be stored on one of our calendars. Stored with us as
 * ORGANIZER, a scheduling server (RFC 6638 section 3.2.1) would mail its ATTENDEE list from us.
 */
export function mayBeAddedToCalendar(organizerUri: string, addresses: string[]): boolean {
  const organizer = CalendarUtils.emailFromParticipantURI(organizerUri);
  if (!organizer) return false;
  return !addresses.some((a) => Utils.emailIsEquivalent(a, organizer));
}

/**
 * The calendars whose events are this account's busy time: not read-only feeds (holidays,
 * fixtures), not calendars switched off in the sidebar, not ones the server says are someone else's.
 */
export function conflictCalendarIds(
  calendars: Calendar[],
  disabledCalendarIds: string[]
): string[] {
  const disabled = new Set(disabledCalendarIds);
  return calendars
    .filter((c) => !c.readOnly && !disabled.has(c.id) && !CalendarUtils.isSomeoneElsesCalendar(c))
    .map((c) => c.id);
}

/**
 * The calendars an unsynced invitation may be added to, and the one preselected. `current`
 * is the user's own pick from an earlier tick, kept while it is still a choice.
 */
export function resolveAddTo({
  rsvp,
  calendars,
  addresses,
  organizerUri,
  current,
}: {
  rsvp: RSVPTargetResolution;
  calendars: Calendar[];
  addresses: string[];
  organizerUri: string;
  current?: Calendar;
}): { choices: Calendar[]; addTo: Calendar } | null {
  if (rsvp.problem !== 'not-on-a-calendar' || !mayBeAddedToCalendar(organizerUri, addresses)) {
    return null;
  }
  const choices = calendars.filter((c) => !c.readOnly && !CalendarUtils.isSomeoneElsesCalendar(c));
  if (!choices.length) return null;
  const addTo =
    (current && choices.find((c) => c.id === current.id)) ||
    choices.find((c) => CalendarUtils.isOwnCalendar(c, addresses)) ||
    choices[0];
  return { choices, addTo };
}

/**
 * What events.rsvp is asked to record beside the emailed REPLY: our own copy when one is
 * certainly ours, the invitation added to `addTo` when we have none, otherwise nothing (the
 * engine would otherwise answer whichever copy of the UID it finds first).
 */
export function rsvpWriteParams({
  rsvp,
  addTo,
}: {
  rsvp: RSVPTargetResolution | undefined;
  addTo: Calendar | undefined;
}): { eventId?: string; calendarId?: string; write?: boolean } {
  if (rsvp && rsvp.target) return { eventId: rsvp.target.event.id };
  if (rsvp && rsvp.problem === 'not-on-a-calendar' && addTo) return { calendarId: addTo.id };
  return { write: false };
}

/** Why a counter-proposal that arrived by email may not be applied to our copy. */
export type CounterProposalProblem =
  /** Our copy of the meeting is not one this account organizes. */
  | 'not-our-meeting'
  /** The message did not come from somebody on the guest list. */
  | 'not-from-a-guest'
  /** Our copy is a series and the proposal names no occurrence, so it would move them all. */
  | 'series-without-occurrence';

/**
 * Whether an emailed COUNTER may move our copy of the meeting: only the organizer revises one
 * (RFC 5546 section 2.1.4) and only a guest counters (section 3.2.7). Both are read off our
 * synced copy, never the attachment, whose UID, ORGANIZER and guests its sender wrote. A
 * counter for a series that names no occurrence is withheld too: applying it would re-base
 * every occurrence on the proposed date, and Thunderbird's invitation bar draws the same line.
 */
export function counterProposalProblem({
  ics,
  senderEmail,
  addresses,
  namesOccurrence = false,
}: {
  ics: string;
  senderEmail: string | null;
  addresses: string[];
  /** Whether the COUNTER carries a RECURRENCE-ID. */
  namesOccurrence?: boolean;
}): CounterProposalProblem | null {
  let event: ICAL.Event;
  try {
    event = CalendarUtils.parseICSString(ics).event;
  } catch (e) {
    return 'not-our-meeting';
  }

  const organizer = event.organizer ? CalendarUtils.emailFromParticipantURI(event.organizer) : null;
  if (!organizer || !addresses.some((a) => Utils.emailIsEquivalent(a, organizer))) {
    return 'not-our-meeting';
  }

  const guests = CalendarUtils.cleanParticipants(event)
    .map((p) => p.email)
    .filter((email): email is string => !!email);
  if (!guests.some((guest) => Utils.emailIsEquivalent(guest, senderEmail))) {
    return 'not-from-a-guest';
  }

  if (event.isRecurring() && !namesOccurrence) {
    return 'series-without-occurrence';
  }

  return null;
}
