import { Monitor, Moon, Sun } from 'lucide-react';
import { useTheme } from 'next-themes';
import { Switch } from '@/components/ui/switch';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { toast } from 'sonner';
import { setEngineSettings, SettingKey, useEngine } from '@/engine';
import { setPreferences, usePreferences, type Density } from '@/lib/preferences';
import { useUndoSendMs } from '@/lib/undo-send';
import { ChoiceSelect } from '../components/ChoiceSelect';
import { engineErrorMessage } from '../engine-call';
import { SettingRow, SettingsSection } from '../components/SettingsSection';
import { SettingsPage } from '../components/SettingsPage';
import { MARK_READ_DELAYS, READING_PANES, undoSendChoices } from './options';
import { SessionSection } from './SessionSection';

export function GeneralSettings() {
  const prefs = usePreferences();
  const { theme, setTheme } = useTheme();
  const client = useEngine();
  const undoSend = useUndoSendMs();

  // UI preferences stay in this browser in v1. The
  // undo-send delay is the engine's when it keeps settings, shared with the desktop; this
  // browser's copy stays the fallback for an engine without them.
  const setUndoSend = (ms: number) => {
    setPreferences({ undoSendMs: ms });
    if (!undoSend.inEngine) return;
    setEngineSettings(client, { [SettingKey.UndoSend]: ms }).catch((err: unknown) =>
      toast.error(`Couldn't save the undo-send delay: ${engineErrorMessage(err)}`),
    );
  };

  return (
    <SettingsPage title="General" description="Saved in this browser.">
      <SettingsSection title="Appearance">
        <SettingRow label="Theme" description="System follows your operating system.">
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            spacing={0}
            value={theme ?? 'system'}
            onValueChange={(v) => v && setTheme(v)}
            aria-label="Theme"
          >
            <ToggleGroupItem value="light" aria-label="Light">
              <Sun /> Light
            </ToggleGroupItem>
            <ToggleGroupItem value="dark" aria-label="Dark">
              <Moon /> Dark
            </ToggleGroupItem>
            <ToggleGroupItem value="system" aria-label="System">
              <Monitor /> System
            </ToggleGroupItem>
          </ToggleGroup>
        </SettingRow>
        <SettingRow label="Density" description="Compact fits one line per thread.">
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            spacing={0}
            value={prefs.density}
            onValueChange={(v) => v && setPreferences({ density: v as Density })}
            aria-label="Density"
          >
            <ToggleGroupItem value="compact">Compact</ToggleGroupItem>
            <ToggleGroupItem value="comfortable">Comfortable</ToggleGroupItem>
          </ToggleGroup>
        </SettingRow>
        <SettingRow
          label="Reading pane"
          htmlFor="pref-reading-pane"
          description="Off opens a thread in place of the list."
        >
          <ChoiceSelect
            id="pref-reading-pane"
            value={prefs.readingPane}
            choices={READING_PANES}
            onChange={(readingPane) => setPreferences({ readingPane })}
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection title="Reading">
        <SettingRow label="Mark as read" htmlFor="pref-mark-read">
          <ChoiceSelect
            id="pref-mark-read"
            value={prefs.markReadDelayMs}
            choices={MARK_READ_DELAYS}
            onChange={(ms) => setPreferences({ markReadDelayMs: ms, markReadOnOpen: ms >= 0 })}
          />
        </SettingRow>
        <SettingRow
          label="Load remote images"
          htmlFor="pref-images"
          description="Remote images can tell the sender you opened a message."
        >
          <Switch
            id="pref-images"
            checked={prefs.loadRemoteImages}
            onCheckedChange={(on) => setPreferences({ loadRemoteImages: on })}
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSection title="Sending">
        <SettingRow
          label="Undo send"
          htmlFor="pref-undo-send"
          description={
            undoSend.inEngine
              ? 'Hold a sent message this long so you can take it back. Saved in the engine, so the desktop uses it too.'
              : 'Hold a sent message this long so you can take it back.'
          }
        >
          <ChoiceSelect
            id="pref-undo-send"
            value={undoSend.ms}
            choices={undoSendChoices(undoSend.ms)}
            onChange={setUndoSend}
          />
        </SettingRow>
      </SettingsSection>

      <SessionSection />
    </SettingsPage>
  );
}
