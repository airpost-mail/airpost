import { Ellipsis, PlugZap, RotateCcw, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import type { AccountJSON, ProcessStateJSON } from '@/engine';
import { ConfirmDialog } from '../components/ConfirmDialog';
import { Status } from '../components/StatusDot';
import { engineErrorMessage } from '../engine-call';
import { useRemoveAccount, useResetAccount, useTestAccount } from './actions';
import { accountStatus } from './status';

type Pending = 'reset' | 'remove' | null;

export function AccountRow({
  account,
  process,
}: {
  account: AccountJSON;
  process?: ProcessStateJSON;
}) {
  const [confirm, setConfirm] = useState<Pending>(null);
  const test = useTestAccount();
  const reset = useResetAccount();
  const remove = useRemoveAccount();
  const status = accountStatus(account, process);
  const who = account.label || account.emailAddress;

  const runTest = () =>
    test.mutate(account, {
      onSuccess: () => toast(`Connected to ${account.settings.imap_host ?? account.emailAddress}`),
      onError: (err) =>
        toast.error(`Couldn’t connect ${who}`, { description: engineErrorMessage(err) }),
    });

  const runReset = () =>
    reset.mutate(account.id, {
      onSuccess: () => {
        setConfirm(null);
        toast(`Resyncing ${who}`);
      },
      onError: (err) => toast.error(engineErrorMessage(err)),
    });

  const runRemove = () =>
    remove.mutate(account.id, {
      onSuccess: () => {
        setConfirm(null);
        toast(`Removed ${who}`);
      },
      onError: (err) => toast.error(engineErrorMessage(err)),
    });

  return (
    <div className="flex items-center gap-3 px-4 py-3">
      <span
        aria-hidden
        className="size-1.5 shrink-0 rounded-full"
        style={{ backgroundColor: account.color }}
      />
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex items-baseline gap-2">
          <span className="truncate text-sm font-medium">{who}</span>
          {account.label && account.label !== account.emailAddress && (
            <span className="truncate text-xs text-muted-foreground">{account.emailAddress}</span>
          )}
        </div>
        <span className="truncate font-mono text-2xs text-muted-foreground">
          {account.provider} · {account.settings.imap_host ?? 'no IMAP host'}
        </span>
      </div>
      <span title={status.detail}>
        <Status tone={status.tone}>{status.label}</Status>
      </span>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-sm" aria-label={`Actions for ${who}`}>
            <Ellipsis />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-44">
          <DropdownMenuItem disabled={test.isPending} onSelect={runTest}>
            <PlugZap /> Test connection
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => setConfirm('reset')}>
            <RotateCcw /> Reset cache
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" onSelect={() => setConfirm('remove')}>
            <Trash2 /> Remove account
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <ConfirmDialog
        open={confirm === 'reset'}
        onOpenChange={(o) => !o && setConfirm(null)}
        title={`Reset ${who}?`}
        description="Airpost deletes this account’s local copy of your mail and downloads it again. Mail on the server is not touched."
        confirmLabel="Reset cache"
        pending={reset.isPending}
        onConfirm={runReset}
      />
      <ConfirmDialog
        open={confirm === 'remove'}
        onOpenChange={(o) => !o && setConfirm(null)}
        title={`Remove ${who}?`}
        description="Airpost stops syncing it and deletes its local mail. Mail on the server is not touched; you can add the account again later."
        confirmLabel="Remove account"
        destructive
        pending={remove.isPending}
        onConfirm={runRemove}
      />
    </div>
  );
}
