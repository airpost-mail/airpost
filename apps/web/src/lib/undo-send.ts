// How long a sent message waits for Undo: the engine's `core.sending.undoSend` when it keeps
// settings (DCP stage 3), so the desktop and every paired browser agree; else, or while the
// engine has no value, this browser's preference.
import { getEngineSetting, SettingKey, useEngineSetting } from '@/engine';
import { getPreferences, usePreferences } from './preferences';

function pick(engine: unknown, local: number): number {
  return typeof engine === 'number' ? engine : local;
}

export function getUndoSendMs(): number {
  return pick(getEngineSetting(SettingKey.UndoSend), getPreferences().undoSendMs);
}

/** The delay, and whether it is saved in the engine rather than this browser. */
export function useUndoSendMs(): { ms: number; inEngine: boolean } {
  const local = usePreferences().undoSendMs;
  const { supported, value } = useEngineSetting(SettingKey.UndoSend);
  return { ms: pick(value, local), inEngine: supported };
}
