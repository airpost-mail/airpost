import { LogOut } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { serverLabel, useSession, useSignOut } from '@/features/auth';
import { SettingRow, SettingsSection } from '../components/SettingsSection';

/** Who is signed in, to which Tern server, and Sign out. */
export function SessionSection() {
  const session = useSession();
  const signOut = useSignOut();
  if (!session) return null;
  const demo = session.method === 'demo';
  const features = session.server.features;
  const enabled = features
    ? Object.entries(features)
        .filter(([, on]) => on)
        .map(([name]) => name)
    : [];

  return (
    <SettingsSection title="Session">
      <SettingRow label="Signed in as" description={session.user.name || undefined}>
        <span className="truncate text-sm">{session.user.email}</span>
      </SettingRow>
      <SettingRow
        label="Tern server"
        description={
          demo
            ? 'The in-browser demo engine with sample mail.'
            : enabled.length
              ? `Server features: ${enabled.join(', ')}.`
              : 'No server features are on.'
        }
      >
        <span className="truncate font-mono text-xs text-muted-foreground">
          {demo ? 'demo' : serverLabel(session.server.url)}
        </span>
      </SettingRow>
      <SettingRow label="Sign out" description="Removes this browser's access to your mail.">
        <Button variant="outline" size="sm" onClick={() => void signOut()}>
          <LogOut /> Sign out
        </Button>
      </SettingRow>
    </SettingsSection>
  );
}
