// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { Actions, Message, SendDraftTask } from 'mailspring-exports';
import { localizerFor } from '../../src/intl/localize';
import { resurfacingBodies } from '../../src/engine/resurfacing';

describe('the resurfacing messages', function () {
  it('are, untranslated, the engine fallbacks (tern-sync expiration_executor)', function () {
    const bodies = resurfacingBodies((en) => en);
    expect(bodies.snoozeBody).toBe(
      '<p><strong>Tern Reminder:</strong> This thread has been moved to the top of your inbox by Tern.</p><p>--The Tern Team</p>'
    );
    expect(bodies.reminderBody).toBe(
      '<p><strong>Tern Reminder:</strong> This thread has been moved to the top of your inbox by Tern because no one has replied to your message.</p><p>--The Tern Team</p>'
    );
  });

  it('are written in the configured language, not the running one', function () {
    expect(resurfacingBodies(localizerFor('de')).snoozeBody).toContain('Tern-Erinnerung');
    expect(localizerFor('xx-none')('The Tern Team')).toBe('The Tern Team');
  });

  it('report nothing to the user when a silent send fails', function () {
    spyOn(Actions, 'draftDeliveryFailed');
    spyOn(console, 'warn');
    const draft = new Message({ headerMessageId: 'h1', accountId: 'a1', threadId: 't1' });
    const silent = new SendDraftTask({});
    silent.draft = draft;
    silent.silent = true;
    silent.onError({ key: 'send-failed', debuginfo: 'ErrorConnection' });
    expect(Actions.draftDeliveryFailed).not.toHaveBeenCalled();

    const shown = new SendDraftTask({});
    shown.draft = draft;
    shown.onError({ key: 'send-failed', debuginfo: 'ErrorConnection' });
    expect(Actions.draftDeliveryFailed).toHaveBeenCalled();
  });
});
