import {
  AccountStore,
  Actions,
  CalendarUtils,
  DatabaseStore,
  Event,
  ICSParticipantStatus,
  localized,
} from 'mailspring-exports';
import React from 'react';
import { ProposeTimePopover } from 'mailspring-component-kit';
import { EventOccurrence, occurrenceStartUnix, occurrenceEndUnix } from '../data-source';
import { parseEventIdFromOccurrence } from '../drag-utils';
import { showRecurringEventDialog } from './recurring-dialog';
import { answerEvent, refusalKey } from '../../../engine/pim';

/** Whether this occurrence is an invitation we can answer, rather than one we sent. */
export function canRespondToEvent(occurrence: EventOccurrence): boolean {
  const me = myAttendeeEmail(occurrence);
  if (!me) return false;

  // The organizer doesn't RSVP to their own meeting; they change it.
  const organizer =
    occurrence.organizer && CalendarUtils.emailFromParticipantURI(occurrence.organizer.email);
  return !organizer || organizer.toLowerCase() !== me;
}

/**
 * Our address on this event's guest list, lowercased. Scoped to the event's own account: another
 * connected account may be a guest too, and selfParticipant would not find them.
 */
export function myAttendeeEmail(occurrence: EventOccurrence): string | null {
  for (const attendee of occurrence.attendees) {
    if (!attendee.email) continue;
    const account = AccountStore.accountForEmail(attendee.email);
    if (account && account.id === occurrence.accountId) {
      return attendee.email.toLowerCase();
    }
  }
  return null;
}

/** Our current participation status for this occurrence, if we're on the guest list. */
export function myParticipationStatus(occurrence: EventOccurrence): string | null {
  const me = myAttendeeEmail(occurrence);
  if (!me) return null;
  const mine = occurrence.attendees.find((a) => a.email && a.email.toLowerCase() === me);
  return mine ? (mine.partstat || 'NEEDS-ACTION').toUpperCase() : null;
}

/**
 * Answers an invitation from the calendar: our PARTSTAT on our copy (RFC 6638 section 3.2.5)
 * and an emailed REPLY to the organizer, which goes even when the copy cannot be written. On a
 * series it first asks, as Google Calendar and Delete do, whether the answer is for this
 * occurrence or for all of them; one occurrence is answered on its own VEVENT and in a REPLY
 * that names it (RFC 5546 section 3.2.3). The engine builds both (events.rsvp).
 */
export async function respondToCalendarEvent(
  occurrence: EventOccurrence,
  status: ICSParticipantStatus
): Promise<void> {
  const eventId = parseEventIdFromOccurrence(occurrence.id);
  const event = await DatabaseStore.find<Event>(Event, eventId);
  if (!event) {
    console.warn(`Calendar RSVP: could not find event ${eventId}`);
    return;
  }
  if (!myAttendeeEmail(occurrence)) {
    AppEnv.showErrorDialog(
      localized("You're not on this event's guest list, so there's no RSVP to give.")
    );
    return;
  }

  let thisOccurrenceOnly = false;
  if (occurrence.isRecurring && !event.isRecurrenceException()) {
    const choice = await showRecurringEventDialog('answer', occurrence.title);
    if (choice === 'cancel') return;
    thisOccurrenceOnly = choice === 'this-occurrence';
  }

  try {
    await answerEvent({
      accountId: event.accountId,
      eventId: event.id,
      status,
      occurrenceStart: thisOccurrenceOnly ? occurrenceSlot(occurrence) : undefined,
    });
  } catch (e) {
    showRefusal(e, localized("You're not on this event's guest list, so there's no RSVP to give."));
  }
}

/** The slot an occurrence fills: its RECURRENCE-ID instant, else its start. */
export function occurrenceSlot(occurrence: EventOccurrence): number {
  return occurrence.recurrenceIdStart ?? occurrenceStartUnix(occurrence);
}

function showRefusal(err: any, notInvited: string) {
  const key = refusalKey(err);
  if (key === 'not-invited') {
    AppEnv.showErrorDialog(notInvited);
  } else if (key === 'no-organizer') {
    AppEnv.showErrorDialog(
      localized("This event has no organizer, so there's nobody to propose a new time to.")
    );
  } else if (key === 'invalid-ics') {
    AppEnv.showErrorDialog(localized("Sorry, this event's data could not be read."));
  } else {
    console.warn(`Calendar RSVP: ${err.message}`);
    AppEnv.showErrorDialog(localized("Sorry, we couldn't send your response to this event."));
  }
}

/**
 * Counter-proposes another time for a meeting we were invited to (RFC 5546 section 3.2.7),
 * from the calendar where the accepted invitation lives. Nothing is written locally: the event
 * changes only when the organizer accepts and sends the update back.
 */
export async function proposeNewTimeForCalendarEvent(
  occurrence: EventOccurrence,
  proposal: { start: Date; end: Date; comment: string }
): Promise<void> {
  const eventId = parseEventIdFromOccurrence(occurrence.id);
  const event = await DatabaseStore.find<Event>(Event, eventId);
  if (!event) {
    console.warn(`Calendar counter-proposal: could not find event ${eventId}`);
    return;
  }
  const notInvited = localized(
    "You're not on this event's guest list, so there's no new time to propose."
  );
  if (!myAttendeeEmail(occurrence)) {
    AppEnv.showErrorDialog(notInvited);
    return;
  }

  try {
    await answerEvent({
      accountId: event.accountId,
      eventId: event.id,
      status: 'TENTATIVE',
      counter: {
        start: Math.round(proposal.start.getTime() / 1000),
        end: Math.round(proposal.end.getTime() / 1000),
      },
      comment: proposal.comment || undefined,
      occurrenceStart:
        occurrence.isRecurring || occurrence.isException ? occurrenceSlot(occurrence) : undefined,
      subject: localized('New time proposed: %@', occurrence.title),
    });
  } catch (e) {
    showRefusal(e, notInvited);
  }
}

/**
 * Opens the time picker for a counter-proposal, anchored on the event in the grid, or on the
 * window's centre when the grid has scrolled it out of view.
 */
export function openProposeNewTimePopover(occurrence: EventOccurrence): void {
  const eventEl = document.getElementById(occurrence.id);
  Actions.openPopover(
    React.createElement(ProposeTimePopover, {
      start: occurrenceStartUnix(occurrence),
      end: occurrenceEndUnix(occurrence),
      isAllDay: occurrence.isAllDay,
      onPropose: (proposal: { start: Date; end: Date; comment: string }) =>
        proposeNewTimeForCalendarEvent(occurrence, proposal),
    }),
    {
      originRect: eventEl
        ? eventEl.getBoundingClientRect()
        : new DOMRect(window.innerWidth / 2, window.innerHeight / 2, 2, 2),
      direction: 'right',
      fallbackDirection: 'left',
    }
  );
}

/**
 * What a guest gets for trying to move a meeting: only the organizer reschedules one (RFC 5546
 * section 2.1.4), so the dialog offers the counter-proposal that stands in for the move.
 */
export function offerCounterInsteadOfMove(occurrence: EventOccurrence): void {
  const response = require('@electron/remote').dialog.showMessageBoxSync({
    type: 'info',
    buttons: [localized('Propose a new time'), localized('Cancel')],
    defaultId: 0,
    cancelId: 1,
    title: localized("This meeting can't be rescheduled"),
    message: localized('Only the organizer can move "%@".', occurrence.title),
    detail: localized('You can propose a new time to the organizer instead.'),
  });
  if (response === 0) {
    openProposeNewTimePopover(occurrence);
  }
}
