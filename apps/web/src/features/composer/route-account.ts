import type { AnyRouter } from '@tanstack/react-router';

/** The account in the current URL, if any. Read once when a window builds its draft. */
export function routeAccountId(router: AnyRouter | undefined): string | undefined {
  for (const m of router?.state.matches ?? []) {
    const id = (m.params as Record<string, string | undefined>).accountId;
    if (id) return id;
  }
  return undefined;
}
