import { Link, useNavigate } from '@tanstack/react-router';
import { ArrowLeft } from 'lucide-react';
import { useState } from 'react';
import { SettingsPage } from '../components/SettingsPage';
import { AccountForm } from './AccountForm';
import { ProviderPicker } from './ProviderPicker';
import { presetById, type ProviderPreset } from './presets';

export function AddAccountView({ provider }: { provider?: string }) {
  const navigate = useNavigate();
  const [preset, setPreset] = useState<ProviderPreset | undefined>(() => presetById(provider));

  return (
    <SettingsPage
      title="Add an account"
      description={
        preset
          ? `${preset.name}. The engine checks the connection before it adds the account.`
          : 'Pick your provider. Tern connects over IMAP and SMTP.'
      }
      actions={
        preset ? (
          <button
            type="button"
            onClick={() => setPreset(undefined)}
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" /> Providers
          </button>
        ) : (
          <Link
            to="/settings/accounts"
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" /> Accounts
          </Link>
        )
      }
    >
      {preset ? (
        <AccountForm
          key={preset.id}
          preset={preset}
          onAdded={() => void navigate({ to: '/settings/accounts' })}
        />
      ) : (
        <ProviderPicker onPick={setPreset} />
      )}
    </SettingsPage>
  );
}
