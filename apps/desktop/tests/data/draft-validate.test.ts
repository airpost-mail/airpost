// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import { Contact } from '../../src/data/models/contact';
import { Message } from '../../src/data/models/message';
import { DraftEditingSession } from '../../src/data/drafts/editing-session';
import EngineEvents from '../../src/engine/client/events';
import { ComposerExtension } from '../../src/packages/composer-extension';
import * as ExtensionRegistry from '../../src/packages/extension-registry';

const HMID = 'draft-1@airpost';

class WarningExtension extends ComposerExtension {
  static warningsForSending() {
    return ['A plugin warning'];
  }
}

describe('DraftEditingSession.validateForSending', () => {
  let session: DraftEditingSession;
  let sent: [string, any][];

  beforeEach(() => {
    session = new DraftEditingSession(
      HMID,
      new Message({
        id: 'd1',
        accountId: 'a1',
        headerMessageId: HMID,
        draft: true,
        to: [new Contact({ name: 'Carol Jones', email: 'carol@example.com' })],
        body: '<p>Hi Dave</p>',
      })
    );
    sent = [];
    spyOn(EngineEvents, 'request').andCallFake((method: string, params: any) => {
      sent.push([method, params]);
      return Promise.resolve({
        errors: [{ code: 'unsendable-from', message: 'English', group: 'misc' }],
        warnings: [
          {
            code: 'salutation-mismatch',
            message: `The message is addressed to a name that doesn't appear to be a recipient ("Dave")`,
            group: 'recipients',
          },
          { code: 'blank-subject', message: 'The subject field is blank.', group: 'misc' },
          { code: 'template-variable', message: 'Fill in: who.', group: 'misc' },
        ],
      });
    });
    localStorage.setItem('recipientWarningBlacklist', JSON.stringify(['ann@example.com']));
  });

  afterEach(() => {
    localStorage.removeItem('recipientWarningBlacklist');
    ExtensionRegistry.Composer.unregister(WarningExtension);
    session.teardown();
  });

  it("saves the draft, asks the engine, and adds the plugins' warnings", async () => {
    spyOn(session.changes, 'commit').andReturn(Promise.resolve());
    ExtensionRegistry.Composer.register(WarningExtension);
    const result = await session.validateForSending();
    expect(session.changes.commit).toHaveBeenCalled();
    expect(sent).toEqual([
      [
        'draft.validate',
        { accountId: 'a1', headerMessageId: HMID, skipSalutationFor: ['ann@example.com'] },
      ],
    ]);
    expect(result).toEqual({
      recipientErrors: [],
      recipientWarnings: [
        `The message is addressed to a name that doesn't appear to be a recipient ("Dave")`,
      ],
      miscErrors: [DraftEditingSession.unsendableFromAddressMessage()],
      miscWarnings: ['The subject field is blank.', 'Fill in: who.', 'A plugin warning'],
    });
  });

  it('no longer validates in the window', () => {
    expect(() => session.validateDraftForSending()).toThrow();
    expect(() => session.validateDraftRecipients()).toThrow();
  });
});
