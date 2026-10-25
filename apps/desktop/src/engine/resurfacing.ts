// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The bodies of the hidden messages that bring a thread back to the top of the inbox: an unsnooze
(thread-snooze) and a reminder nobody answered (send-reminders). The engine's expiration
executor sends them, with the texts the settings mirror writes to `resurfacing.snoozeBody` /
`resurfacing.reminderBody`.
With no translation these are the engine's English fallbacks.
*/
import { localized } from '../intl/localize';

export const SNOOZE_BODY_KEY = 'resurfacing.snoozeBody';
export const REMINDER_BODY_KEY = 'resurfacing.reminderBody';

export function resurfacingBodies(t: (en: string) => string = localized) {
  const lead = `<p><strong>${t('Tern Reminder')}:</strong> `;
  const team = `<p>--${t('The Tern Team')}</p>`;
  return {
    snoozeBody: `${lead}${t('This thread has been moved to the top of your inbox by Tern.')}</p>${team}`,
    reminderBody: `${lead}${t(
      'This thread has been moved to the top of your inbox by Tern because no one has replied to your message.'
    )}</p>${team}`,
  };
}
