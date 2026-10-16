// Owned by the search feature.
import { createFileRoute } from '@tanstack/react-router';
import { SearchView, validateSearchRouteSearch } from '@/features/search';

export const Route = createFileRoute('/_app/search')({
  validateSearch: validateSearchRouteSearch,
  component: SearchRoute,
});

function SearchRoute() {
  const { q, account, thread, threadAccount } = Route.useSearch();
  return (
    <SearchView
      query={q ?? ''}
      accountId={account}
      openThread={thread && threadAccount ? { accountId: threadAccount, threadId: thread } : null}
    />
  );
}
