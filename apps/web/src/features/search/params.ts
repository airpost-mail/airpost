/** `/search` URL state. `thread` + `threadAccount` is the thread open in the reading pane. */
export interface SearchRouteSearch {
  q?: string;
  account?: string;
  thread?: string;
  threadAccount?: string;
}

const str = (v: unknown) => (typeof v === 'string' && v !== '' ? v : undefined);

export function validateSearchRouteSearch(s: Record<string, unknown>): SearchRouteSearch {
  return {
    q: str(s.q),
    account: str(s.account),
    thread: str(s.thread),
    threadAccount: str(s.threadAccount),
  };
}
