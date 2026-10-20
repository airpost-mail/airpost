// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { DraftStore, Message } from 'mailspring-exports';
import { EngineEvents } from '../../../src/engine/client/events';
import { Feature } from '../../../src/engine/protocol';
import { handleMetadataExpiration } from '../lib/main';
import { PLUGIN_ID } from '../lib/constants';

function dueDraft(value: Record<string, unknown>) {
  return new Message({
    id: 'd1',
    accountId: 'a1',
    headerMessageId: 'h1',
    draft: true,
  }).directlyAttachMetadata(PLUGIN_ID, {
    expiration: new Date(Date.now() - 1000),
    ...value,
  }) as Message;
}

async function expire(message: Message, engine: EngineEvents) {
  handleMetadataExpiration(
    { type: 'metadata-expiration', objectClass: Message.name, objects: [message] },
    engine
  );
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

describe('send-later on metadata-expiration', function () {
  let engine: EngineEvents;

  beforeEach(() => {
    engine = new EngineEvents();
    spyOn(DraftStore, 'sessionForClientId').andReturn(Promise.resolve({ draft: () => null }));
  });

  it('sends the draft itself when no engine runs the expiration executor', async () => {
    await expire(dueDraft({}), engine);
    expect(DraftStore.sessionForClientId).toHaveBeenCalledWith('h1');
  });

  it('leaves the draft to an engine that runs the executor', async () => {
    engine.connected([Feature.ExpirationExecutor], () => Promise.resolve(null));
    await expire(dueDraft({}), engine);
    expect(DraftStore.sessionForClientId).not.toHaveBeenCalled();
  });

  it('sends a draft whose send action the engine does not have', async () => {
    engine.connected([Feature.ExpirationExecutor], () => Promise.resolve(null));
    await expire(dueDraft({ actionKey: 'send-and-snooze' }), engine);
    expect(DraftStore.sessionForClientId).toHaveBeenCalledWith('h1');
  });

  it('leaves an engine send action to the engine', async () => {
    engine.connected([Feature.ExpirationExecutor], () => Promise.resolve(null));
    await expire(dueDraft({ actionKey: 'send-and-archive' }), engine);
    expect(DraftStore.sessionForClientId).not.toHaveBeenCalled();
  });

  it('still finalizes a draft marked clientFinalize', async () => {
    engine.connected([Feature.ExpirationExecutor], () => Promise.resolve(null));
    await expire(dueDraft({ clientFinalize: true }), engine);
    expect(DraftStore.sessionForClientId).toHaveBeenCalledWith('h1');
  });
});
