// Search box, results page and palette against the mock engine, in a route tree shaped
// like the app's (`/_app/search`).
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from '@tanstack/react-router';
import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';
import { EngineProvider, MockEngine } from '@/engine';
import { installShortcuts, resetShortcuts } from '@/lib/shortcuts';
import { CommandPalette } from './CommandPalette';
import { validateSearchRouteSearch } from './params';
import { SearchBox } from './SearchBox';
import { SearchView } from './SearchView';
import { setCommandPaletteOpen } from './store';

let uninstall: (() => void) | null = null;
afterEach(() => {
  uninstall?.();
  uninstall = null;
  resetShortcuts();
  act(() => setCommandPaletteOpen(false));
});

function renderAt(url: string) {
  const engine = new MockEngine({ latency: 0 });
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createRootRoute({
    component: () => (
      <>
        <SearchBox />
        <Outlet />
        <CommandPalette />
      </>
    ),
  });
  const app = createRoute({ getParentRoute: () => root, id: '_app', component: Outlet });
  const search = createRoute({
    getParentRoute: () => app,
    path: 'search',
    validateSearch: validateSearchRouteSearch,
    component: function Search() {
      const { q, account } = search.useSearch();
      return <SearchView query={q ?? ''} accountId={account} />;
    },
  });
  const home = createRoute({ getParentRoute: () => app, path: '/', component: () => <p>Home</p> });
  const router = createRouter({
    routeTree: root.addChildren([app.addChildren([search, home])]),
    history: createMemoryHistory({ initialEntries: [url] }),
  });
  uninstall = installShortcuts(window);
  render(
    <QueryClientProvider client={queryClient}>
      <EngineProvider client={engine}>
        <RouterProvider router={router} />
      </EngineProvider>
    </QueryClientProvider>,
  );
  return { router, q: () => (router.state.location.search as { q?: string }).q };
}

describe('search page', () => {
  it('lists matching threads and shows the query as chips', async () => {
    renderAt('/search?q=invoice');
    const list = await screen.findByRole('list', { name: 'Search results' });
    expect(await within(list).findAllByText(/Invoice/i)).not.toHaveLength(0);
    expect(screen.getByRole('button', { name: 'Remove “invoice”' })).toBeInTheDocument();
    expect(screen.getByRole('combobox', { name: 'Search mail' })).toHaveValue('invoice');
  });

  it('toggles flags and removes chips through the URL', async () => {
    const user = userEvent.setup();
    const { router, q } = renderAt('/search?q=invoice%20from%3Astripe');
    await screen.findByRole('list', { name: 'Search results' });

    await user.click(screen.getByRole('button', { name: /Unread/ }));
    await waitFor(() => expect(q()).toBe('invoice from:stripe is:unread'));

    await user.click(screen.getByRole('button', { name: 'Remove From stripe' }));
    await waitFor(() => expect(q()).toBe('invoice is:unread'));
    expect(router.state.location.pathname).toBe('/search');
  });

  it('asks for company when only dates are given', async () => {
    renderAt('/search?q=after%3A7d');
    expect(await screen.findByText('Almost there')).toBeInTheDocument();
  });

  it('says so when nothing matches', async () => {
    renderAt('/search?q=zzzznotathing');
    expect(await screen.findByText('No sightings')).toBeInTheDocument();
  });
});

describe('search box', () => {
  it('focuses on / and submits to the results page', async () => {
    const user = userEvent.setup();
    const { router, q } = renderAt('/');
    await screen.findByText('Home');
    const box = screen.getByRole('combobox', { name: 'Search mail' });

    await user.keyboard('/');
    await waitFor(() => expect(box).toHaveFocus());
    await user.type(box, 'kickoff{Enter}');
    await waitFor(() => expect(router.state.location.pathname).toBe('/search'));
    expect(q()).toBe('kickoff');
  });

  it('suggests operator values', async () => {
    const user = userEvent.setup();
    renderAt('/');
    const box = await screen.findByRole('combobox', { name: 'Search mail' });
    await user.click(box);
    await user.type(box, 'is:');
    const listbox = screen.getByRole('listbox', { name: 'Search suggestions' });
    await user.click(within(listbox).getByRole('option', { name: /^unread/ }));
    expect(box).toHaveValue('is:unread ');
  });
});

describe('command palette', () => {
  it('filters commands and offers a mail search', async () => {
    const user = userEvent.setup();
    renderAt('/');
    await screen.findByText('Home');
    act(() => setCommandPaletteOpen(true));
    const input = await screen.findByPlaceholderText('Search mail or type a command');
    await user.type(input, 'sett');
    expect(screen.getByRole('option', { name: /Settings/ })).toBeInTheDocument();
    expect(screen.getByRole('option', { name: /Search mail for “sett”/ })).toBeInTheDocument();
    expect(screen.queryByRole('option', { name: /Compose/ })).not.toBeInTheDocument();
  });

  it('shows live mail results and opens them in the search view', async () => {
    const user = userEvent.setup();
    const { router, q } = renderAt('/');
    await screen.findByText('Home');
    act(() => setCommandPaletteOpen(true));
    const input = await screen.findByPlaceholderText('Search mail or type a command');
    await user.type(input, 'Invoice 2026-031');
    const hit = await screen.findByRole('option', { name: /Invoice 2026-031 paid/ });
    await user.click(hit);
    await waitFor(() => expect(router.state.location.pathname).toBe('/search'));
    expect(q()).toBe('Invoice 2026-031');
    expect((router.state.location.search as { thread?: string }).thread).toBeTruthy();
  });
});
