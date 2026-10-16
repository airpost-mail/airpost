import { Monitor, Moon, Sun } from 'lucide-react';
import { useTheme } from 'next-themes';
import { Switch } from '@/components/ui/switch';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { setPreferences, usePreferences, type Density } from '@/lib/preferences';
import { ChoiceSelect } from '../components/ChoiceSelect';
import { SettingRow, SettingsSection } from '../components/SettingsSection';
import { SettingsPage } from '../components/SettingsPage';
import { MARK_READ_DELAYS, READING_PANES, UNDO_SEND_DELAYS } from './options';
import { SessionSection } from './SessionSection';

export function GeneralSettings() {
  const prefs = usePreferences();
  const { theme, setTheme } = useTheme();

  // TODO(engine): preferences live in this browser only; sync them once the engine or
  // backend stores per-user settings.
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
          description="Hold a sent message this long so you can take it back."
        >
          <ChoiceSelect
            id="pref-undo-send"
            value={prefs.undoSendMs}
            choices={UNDO_SEND_DELAYS}
            onChange={(undoSendMs) => setPreferences({ undoSendMs })}
          />
        </SettingRow>
      </SettingsSection>

      <SessionSection />
    </SettingsPage>
  );
}
