import { useMutation, useQueryClient } from '@tanstack/react-query';
import { engineKeys, useEngine, type AccountJSON, type AccountTestResult } from '@/engine';
import { callEngine } from '../engine-call';

export class AccountTestError extends Error {
  readonly log?: string;
  constructor(result: AccountTestResult) {
    super(result.error || 'The server rejected the connection.');
    this.name = 'AccountTestError';
    this.log = result.log;
  }
}

/** Tests the settings with `account.test`, then adds the account. */
export function useAddAccount() {
  const client = useEngine();
  return useMutation({
    mutationFn: async (account: AccountJSON) => {
      const test = await callEngine(client, 'account.test', { account });
      if (test.error) throw new AccountTestError(test);
      // The Account delta from `account.add` updates the accounts query (engine/cache.ts).
      return callEngine(client, 'account.add', { account });
    },
  });
}

export function useTestAccount() {
  const client = useEngine();
  return useMutation({
    // TODO(engine): `account.test` takes full settings, but `account.list` never hands
    // passwords to a browser; testing a stored account needs `account.test {accountId}`,
    // which the engine doesn't have.
    mutationFn: async (account: AccountJSON) => {
      const result = await callEngine(client, 'account.test', { account });
      if (result.error) throw new AccountTestError(result);
      return result;
    },
  });
}

export function useResetAccount() {
  const client = useEngine();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => callEngine(client, 'account.reset', { accountId }),
    onSuccess: (_r, accountId) => {
      // `account.reset` needs a device paired with the admin scope. The provider also
      // refetches when `account.state` goes from starting to running; the mock sends none, so
      // refetch here too.
      for (const key of [
        engineKeys.folders(accountId),
        engineKeys.labels(accountId),
        engineKeys.threadLists(accountId),
        engineKeys.messagesAll(accountId),
      ]) {
        void qc.invalidateQueries({ queryKey: key });
      }
    },
  });
}

export function useRemoveAccount() {
  const client = useEngine();
  return useMutation({
    mutationFn: (accountId: string) => callEngine(client, 'account.remove', { accountId }),
  });
}
