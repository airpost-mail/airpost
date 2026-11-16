import { LogOut } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useServerIdentityState } from '@/engine';
import { isAdmin, useSession, useSignOut } from '@/features/auth';
import { SettingRow, SettingsSection } from '../components/SettingsSection';

/** This browser's pairing, the engine behind the gateway, and Sign out. */
export function SessionSection() {
  const session = useSession();
  const signOut = useSignOut();
  const identity = useServerIdentityState();
  if (!session) return null;
  const { demo, device, engine, gateway } = session;

  return (
    <SettingsSection title="Session">
      <SettingRow
        label="This browser"
        description={
          isAdmin(session)
            ? 'Admin access: can also add accounts and change the engine.'
            : 'Mail access.'
        }
      >
        <span className="truncate text-sm">{device.name}</span>
      </SettingRow>
      <SettingRow
        label="Engine"
        description={
          demo
            ? 'The in-browser demo engine with sample mail.'
            : engine.state === 'up'
              ? `Through the gateway at ${window.location.host} (gateway ${gateway.version}).`
              : "The engine isn't running."
        }
      >
        <span className="truncate font-mono text-xs text-muted-foreground">
          {demo ? 'demo' : (engine.engine ?? engine.state)}
        </span>
      </SettingRow>
      {identity.refreshNeeded && (
        <SettingRow
          label="Airpost server"
          description="The engine's Airpost server rejected its sign-in. Sign the engine in again from the desktop app or the CLI."
        >
          <span className="text-xs text-warning">Signed out</span>
        </SettingRow>
      )}
      <SettingRow label="Sign out" description="Unpairs this browser from the gateway.">
        <Button variant="outline" size="sm" onClick={() => void signOut()}>
          <LogOut /> Sign out
        </Button>
      </SettingRow>
    </SettingsSection>
  );
}
