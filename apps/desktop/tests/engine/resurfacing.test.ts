// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import * as Actions from '../../src/data/actions';
import { Message } from '../../src/data/models/message';
import { SendDraftTask } from '../../src/data/tasks/send-draft';
import { localizerFor } from '../../src/intl/localize';
import { resurfacingBodies } from '../../src/engine/resurfacing';

describe('the resurfacing messages', function () {
  it('are, untranslated, the engine fallbacks (airpost-sync expiration_executor)', function () {
    const bodies = resurfacingBodies((en) => en);
    expect(bodies.snoozeBody).toBe(
      '<p><strong>Airpost Reminder:</strong> This thread has been moved to the top of your inbox by Airpost.</p><p>--The Airpost Team</p>'
    );
    expect(bodies.reminderBody).toBe(
      '<p><strong>Airpost Reminder:</strong> This thread has been moved to the top of your inbox by Airpost because no one has replied to your message.</p><p>--The Airpost Team</p>'
    );
  });

  it('are written in the configured language, not the running one', function () {
    expect(resurfacingBodies(localizerFor('de')).snoozeBody).toContain('Airpost-Erinnerung');
    expect(localizerFor('xx-none')('The Airpost Team')).toBe('The Airpost Team');
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
