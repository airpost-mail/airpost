import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { getUndoSendMs } from '@/lib/undo-send';
import { setPreferences } from '@/lib/preferences';
import { MOCK_FEATURES, MockEngine } from './mock/engine';
import { EngineProvider } from './provider';
import { EngineFeature, RpcErrorCode } from './rpc';
import {
  engineHasSettings,
  getEngineCapabilities,
  getEngineSetting,
  resetEngineState,
  setEngineSettings,
  SettingKey,
} from './settings';

function mount(engine: MockEngine) {
  render(
    <QueryClientProvider client={new QueryClient()}>
      <EngineProvider client={engine}>
        <p>app</p>
      </EngineProvider>
    </QueryClientProvider>,
  );
}

afterEach(() => {
  resetEngineState();
  setPreferences({ undoSendMs: 5000 });
});

describe('engine settings', () => {
  it('reads the engine’s values on connect and follows other connections’ changes', async () => {
    const engine = new MockEngine({ latency: 0 });
    engine.settings.set(SettingKey.UndoSend, 15000);
    mount(engine);
    await waitFor(() => expect(engineHasSettings()).toBe(true));
    expect(getEngineSetting(SettingKey.UndoSend)).toBe(15000);
    expect(getUndoSendMs()).toBe(15000);

    engine.changeSettings({ [SettingKey.UndoSend]: 0 });
    expect(getUndoSendMs()).toBe(0);
    // Cleared there: this browser's value applies again.
    setPreferences({ undoSendMs: 30000 });
    engine.changeSettings({ [SettingKey.UndoSend]: null });
    expect(getEngineSetting(SettingKey.UndoSend)).toBeUndefined();
    expect(getUndoSendMs()).toBe(30000);
  });

  it('writes to the engine and takes a refused write back', async () => {
    const engine = new MockEngine({ latency: 0 });
    mount(engine);
    await waitFor(() => expect(engineHasSettings()).toBe(true));
    await setEngineSettings(engine, { [SettingKey.UndoSend]: 60000 });
    expect(engine.settings.get(SettingKey.UndoSend)).toBe(60000);

    // A device without the admin scope can't set signatures (tern-ipc RELAYED_SETTINGS_KEYS).
    const refused = setEngineSettings(engine, { signatures: {} });
    expect(getEngineSetting('signatures')).toEqual({});
    await expect(refused).rejects.toMatchObject({ code: RpcErrorCode.NotAllowedRelayed });
    expect(getEngineSetting('signatures')).toBeUndefined();
    expect(getEngineSetting(SettingKey.UndoSend)).toBe(60000);
  });

  it('keeps this browser’s values for an engine without settings', async () => {
    const engine = new MockEngine({ latency: 0, features: [EngineFeature.Query] });
    setPreferences({ undoSendMs: 15000 });
    mount(engine);
    await waitFor(() => expect(engine.state).toBe('open'));
    expect(engineHasSettings()).toBe(false);
    expect(getUndoSendMs()).toBe(15000);
    expect(getEngineCapabilities()).toBeNull();
  });

  it('takes capabilities from hello', async () => {
    const engine = new MockEngine({ latency: 0, features: [...MOCK_FEATURES] });
    mount(engine);
    await waitFor(() => expect(getEngineCapabilities()).not.toBeNull());
    expect(getEngineCapabilities()?.server?.readReceipts).toEqual({
      available: false,
      reason: 'needs-tern-server',
    });
  });
});
