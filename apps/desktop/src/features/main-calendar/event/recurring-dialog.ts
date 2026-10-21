import { localized } from '../../../intl/localize';
import { Dialogs } from '../../../platform/app-api';

export type RecurringEventChoice = 'this-occurrence' | 'all-occurrences' | 'cancel';

export type RecurringEventOperation = 'move' | 'resize' | 'delete' | 'edit' | 'answer';

// The dialog doesn't block the window, so a second move or delete could open one alongside the
// first; this chain shows them one after another.
let pending: Promise<unknown> = Promise.resolve();

/**
 * Shows a dialog asking whether to modify this occurrence or all occurrences
 * of a recurring event.
 *
 * @param operation - The type of operation being performed
 * @param eventTitle - The event title for display in the dialog
 * @returns Promise resolving to the user's choice
 */
export function showRecurringEventDialog(
  operation: RecurringEventOperation,
  eventTitle: string
): Promise<RecurringEventChoice> {
  const next = pending.then(() => ask(operation, eventTitle));
  pending = next.catch(() => undefined);
  return next;
}

async function ask(
  operation: RecurringEventOperation,
  eventTitle: string
): Promise<RecurringEventChoice> {
  const operationText: Record<RecurringEventOperation, string> = {
    move: localized('Move'),
    resize: localized('Resize'),
    delete: localized('Delete'),
    edit: localized('Edit'),
    answer: localized('Answer'),
  };

  const operationVerb: Record<RecurringEventOperation, string> = {
    move: localized('move'),
    resize: localized('resize'),
    delete: localized('delete'),
    edit: localized('edit'),
    answer: localized('answer'),
  };

  const response = await Dialogs.ask(
    {
      type: 'question',
      buttons: [
        localized('This occurrence only'),
        localized('All occurrences'),
        localized('Cancel'),
      ],
      defaultId: 0,
      cancelId: 2,
      title: localized('Recurring Event'),
      message: localized('%1$@ recurring event "%2$@"?', operationText[operation], eventTitle),
      detail: localized(
        'This is a recurring event. Do you want to %1$@ only this occurrence, or all occurrences in the series?',
        operationVerb[operation]
      ),
    },
    'none'
  );

  const choices: RecurringEventChoice[] = ['this-occurrence', 'all-occurrences', 'cancel'];
  return choices[response];
}
