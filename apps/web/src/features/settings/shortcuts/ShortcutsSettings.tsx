import { useMemo, useState } from 'react';
import { ShortcutKeys } from '@/components/ShortcutKeys';
import { Input } from '@/components/ui/input';
import { useShortcutList } from '@/lib/shortcuts';
import { SettingRow, SettingsSection } from '../components/SettingsSection';
import { SettingsPage } from '../components/SettingsPage';
import { groupShortcuts } from './group';

export function ShortcutsSettings() {
  const list = useShortcutList();
  const [filter, setFilter] = useState('');
  const groups = useMemo(() => groupShortcuts(list, filter), [list, filter]);

  return (
    <SettingsPage
      title="Shortcuts"
      description="Registered by the views that are open. Press ? anywhere for the same list."
      actions={
        <Input
          type="search"
          placeholder="Filter"
          aria-label="Filter shortcuts"
          className="h-7 w-40"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      }
    >
      {groups.length === 0 && (
        <p className="py-10 text-center text-2xl font-semibold tracking-tight text-muted-foreground">
          Nothing matches
        </p>
      )}
      {groups.map(([group, defs]) => (
        <SettingsSection key={group} title={group}>
          {defs.map((s) => (
            <SettingRow key={s.description} label={s.description} className="py-2">
              <span className="flex gap-2">
                {(Array.isArray(s.keys) ? s.keys : [s.keys]).map((k) => (
                  <ShortcutKeys key={k} keys={k} />
                ))}
              </span>
            </SettingRow>
          ))}
        </SettingsSection>
      ))}
    </SettingsPage>
  );
}
