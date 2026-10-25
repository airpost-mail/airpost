// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { DraftStore, Message } from 'mailspring-exports';
import { handleMetadataExpiration } from '../lib/main';
import { PLUGIN_ID } from '../lib/constants';

function dueDraft(value: Record<string, unknown>, fields: Record<string, unknown> = {}) {
  return new Message({
    id: 'd1',
    accountId: 'a1',
    headerMessageId: 'h1',
    draft: true,
    ...fields,
  }).directlyAttachMetadata(PLUGIN_ID, {
    expiration: new Date(Date.now() - 1000),
    ...value,
  }) as Message;
}

async function expire(message: Message) {
  handleMetadataExpiration({
    type: 'metadata-expiration',
    objectClass: Message.name,
    objects: [message],
  });
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

describe('send-later on metadata-expiration', function () {
  beforeEach(() => {
    spyOn(DraftStore, 'sessionForClientId').andReturn(Promise.resolve({ draft: () => null }));
  });

  it('leaves the draft to the engine', async () => {
    await expire(dueDraft({}));
    expect(DraftStore.sessionForClientId).not.toHaveBeenCalled();
  });

  it('sends a draft whose send action the engine does not have', async () => {
    await expire(dueDraft({ actionKey: 'send-and-snooze' }));
    expect(DraftStore.sessionForClientId).toHaveBeenCalledWith('h1');
  });

  it('leaves an engine send action to the engine', async () => {
    await expire(dueDraft({ actionKey: 'send-and-archive' }));
    expect(DraftStore.sessionForClientId).not.toHaveBeenCalled();
  });

  it('leaves a message that is no longer a draft to the engine', async () => {
    await expire(dueDraft({ actionKey: 'send-and-snooze' }, { draft: false }));
    expect(DraftStore.sessionForClientId).not.toHaveBeenCalled();
  });

  it('still finalizes a draft marked clientFinalize', async () => {
    await expire(dueDraft({ clientFinalize: true }));
    expect(DraftStore.sessionForClientId).toHaveBeenCalledWith('h1');
  });
});
