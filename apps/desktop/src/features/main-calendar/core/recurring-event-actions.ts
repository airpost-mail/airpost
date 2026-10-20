import { Event, localized } from 'mailspring-exports';
import { editEvent } from '../../../engine/pim';
import { showRecurringEventDialog, RecurringEventOperation } from './recurring-event-dialog';

/**
 * Options for modifying an event's times
 */
export interface EventTimeChangeOptions {
  /** The Event model to modify */
  event: Event;
  /**
   * Original occurrence start time (unix seconds): the slot it fills.
   * For a regular occurrence this is its RRULE-generated start time.
   * For an inline exception this must be the RECURRENCE-ID value (i.e.
   * `occurrence.recurrenceIdStart`), NOT the exception's moved DTSTART.
   */
  originalOccurrenceStart: number;
  /** New start time (unix seconds) */
  newStart: number;
  /** New end time (unix seconds) */
  newEnd: number;
  /** Whether this is an all-day event */
  isAllDay: boolean;
  /** Description for the undo toast (e.g., "Move event") */
  description?: string;
  /**
   * True when the occurrence being modified is already an inline exception.
   * When set, `modifyEventWithRecurringSupport` skips the "edit all / this
   * occurrence" dialog and always edits only this exception — matching the
   * behaviour of the popover's `saveEdits` path.
   */
  isException?: boolean;
  /** Whether the occurrence belongs to a series (its `isRecurring`). */
  isRecurring?: boolean;
}

/**
 * Result of an event modification operation
 */
export interface EventModificationResult {
  /** Whether the operation was successful */
  success: boolean;
  /** Whether the operation was cancelled by user */
  cancelled?: boolean;
  /** The modified master event (if applicable) */
  masterEvent?: Event;
}

function timesPatch({ newStart, newEnd, isAllDay }: EventTimeChangeOptions) {
  return { start: newStart, end: newEnd, isAllDay };
}

/**
 * Moves a single event, or a series exception stored as its own row, whose resource also holds
 * the series: the engine edits the override the row names. Undoable.
 */
export function modifySimpleEvent(options: EventTimeChangeOptions) {
  return editEvent({
    event: options.event,
    scope: options.event.isRecurrenceException() ? 'this' : 'all',
    patch: timesPatch(options),
    description: options.description || localized('Edit event'),
  });
}

/**
 * Moves one occurrence of a series: an override (RECURRENCE-ID) in the series' own resource,
 * which the engine adds or, for an occurrence already moved, changes. Undoable.
 */
export function createOccurrenceException(options: EventTimeChangeOptions) {
  return editEvent({
    event: options.event,
    scope: 'this',
    occurrenceStart: options.originalOccurrenceStart,
    patch: timesPatch(options),
    description: options.description || localized('Edit occurrence'),
  });
}

/**
 * Moves every occurrence by the change made to one: the engine shifts the series' DTSTART and
 * its overrides' RECURRENCE-IDs by the same amount. Undoable.
 */
export function modifyAllOccurrences(options: EventTimeChangeOptions) {
  return editEvent({
    event: options.event,
    scope: 'all',
    occurrenceStart: options.originalOccurrenceStart,
    patch: timesPatch(options),
    description: options.description || localized('Edit recurring event'),
  });
}

/**
 * Handles modification of an event, showing the recurring dialog if needed.
 * This is the main entry point for event modifications that need to handle
 * recurring events properly.
 *
 * @param options - The modification options
 * @param operation - The type of operation for the dialog (move, resize, edit)
 * @param eventTitle - The event title for the dialog
 * @returns Result indicating success/cancellation and any created events
 */
export async function modifyEventWithRecurringSupport(
  options: EventTimeChangeOptions,
  operation: RecurringEventOperation,
  eventTitle: string
): Promise<EventModificationResult> {
  const { event, isException, isRecurring } = options;

  if (isRecurring && !event.isRecurrenceException() && !isException) {
    const choice = await showRecurringEventDialog(operation, eventTitle);
    if (choice === 'cancel') {
      return { success: false, cancelled: true };
    }
    if (choice === 'this-occurrence') {
      await createOccurrenceException(options);
    } else {
      await modifyAllOccurrences(options);
    }
    return { success: true, masterEvent: event };
  }

  if (isException && !event.isRecurrenceException()) {
    // An inline exception: `options.event` is the master (parseEventIdFromOccurrence strips the
    // `-e{N}` suffix), and only that exception moves.
    await createOccurrenceException(options);
    return { success: true, masterEvent: event };
  }

  // Non-recurring event, or a standalone exception row (event.isRecurrenceException()), which
  // holds the override itself.
  await modifySimpleEvent(options);
  return { success: true, masterEvent: event };
}
