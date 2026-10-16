import { Link } from '@tanstack/react-router';
import { Plus } from 'lucide-react';
import { EmptyState } from '@/components/EmptyState';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { useAccounts, useProcessState } from '@/engine';
import { SettingsPage } from '../components/SettingsPage';
import { SettingsSection } from '../components/SettingsSection';
import { engineErrorMessage } from '../engine-call';
import { AccountRow } from './AccountRow';

export function AccountsSettings() {
  const { data: accounts, isLoading, error } = useAccounts();
  const process = useProcessState();

  const addButton = (
    <Button size="sm" asChild>
      <Link to="/settings/accounts/new">
        <Plus /> Add account
      </Link>
    </Button>
  );

  return (
    <SettingsPage
      title="Accounts"
      description="Mail accounts the engine syncs."
      actions={accounts?.length ? addButton : undefined}
    >
      {isLoading ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-14" />
          <Skeleton className="h-14" />
        </div>
      ) : error ? (
        <p className="text-sm text-destructive">{engineErrorMessage(error)}</p>
      ) : !accounts?.length ? (
        <EmptyState
          className="h-auto py-16"
          title="No accounts yet"
          description="Add a mail account to start syncing."
          action={addButton}
        />
      ) : (
        <SettingsSection
          title={`${accounts.length} ${accounts.length === 1 ? 'account' : 'accounts'}`}
        >
          {accounts.map((a) => (
            <AccountRow key={a.id} account={a} process={process[a.id]} />
          ))}
        </SettingsSection>
      )}
    </SettingsPage>
  );
}
