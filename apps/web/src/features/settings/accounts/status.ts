import type { AccountJSON, ProcessStateJSON } from '@/engine';
import type { Tone } from '../components/StatusDot';

export function accountStatus(
  account: AccountJSON,
  process?: ProcessStateJSON,
): { tone: Tone; label: string; detail?: string } {
  if (account.syncState === 'invalid') {
    return { tone: 'destructive', label: 'Sign-in failed', detail: account.syncError?.error };
  }
  if (account.syncState === 'sync_error') {
    return { tone: 'warning', label: 'Sync error', detail: account.syncError?.error };
  }
  if (process?.connectionError) return { tone: 'warning', label: 'Offline' };
  return { tone: 'success', label: 'Syncing' };
}
