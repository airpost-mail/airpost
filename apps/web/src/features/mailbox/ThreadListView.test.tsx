import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
  useParams,
} from '@tanstack/react-router';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import { EngineProvider, MOCK_ACCOUNT_IDS, MockEngine } from '@/engine';
import { installShortcuts, isMac, resetShortcuts } from '@/lib/shortcuts';
import { ThreadListView } from './ThreadListView';

const G = MOCK_ACCOUNT_IDS.gmail;

function setup(path = `/${G}/g-inbox`, features?: string[]) {
  const engine = new MockEngine({ latency: 0, features });
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createRootRoute({ component: Outlet });
  const folder = createRoute({
    getParentRoute: () => root,
    path: '$accountId/$folderId',
    component: function Folder() {
      const { accountId, folderId } = folder.useParams();
      return (
        <>
          <ThreadListView accountId={accountId} folderId={folderId} />
          <Outlet />
        </>
      );
    },
  });
  const thread = createRoute({
    getParentRoute: () => folder,
    path: '$threadId',
    component: function Thread() {
      return <output aria-label="open thread">{useParams({ strict: false }).threadId}</output>;
    },
  });
  const router = createRouter({
    routeTree: root.addChildren([folder.addChildren([thread])]),
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  render(
    <QueryClientProvider client={queryClient}>
      <EngineProvider client={engine}>
        <TooltipProvider>
          <RouterProvider router={router} />
        </TooltipProvider>
      </EngineProvider>
    </QueryClientProvider>,
  );
  return { engine, router };
}

const rows = () => within(screen.getByRole('list', { name: 'Inbox' })).getAllByRole('listitem');
const cursorRow = () => rows().find((r) => r.hasAttribute('data-cursor'));
const checkedIds = () =>
  rows()
    .filter((r) => r.hasAttribute('data-checked'))
    .map((r) => r.dataset.threadId);

let uninstall: () => void;
beforeEach(() => {
  // The router restores scroll on navigation; jsdom has no layout.
  window.scrollTo = () => {};
  uninstall = installShortcuts(window);
});
afterEach(() => {
  uninstall();
  resetShortcuts();
});

describe('ThreadListView', () => {
  it('lists the inbox newest first with unread rows marked', async () => {
    setup();
    await waitFor(() => expect(rows().length).toBeGreaterThan(3));
    const times = rows().map((r) => r.querySelector('time')!.getAttribute('datetime')!);
    expect([...times].sort().reverse()).toEqual(times);
    expect(rows().some((r) => r.querySelector('.font-semibold'))).toBe(true);
  });

  it('moves the cursor with j/k and opens with enter', async () => {
    const user = userEvent.setup();
    const { router } = setup();
    await waitFor(() => expect(rows().length).toBeGreaterThan(3));
    const [first, second] = rows().map((r) => r.dataset.threadId);

    await user.keyboard('j');
    expect(cursorRow()?.dataset.threadId).toBe(first);
    await user.keyboard('jjk');
    expect(cursorRow()?.dataset.threadId).toBe(second);

    await user.keyboard('{Enter}');
    await waitFor(() => expect(screen.getByLabelText('open thread')).toHaveTextContent(second!));
    expect(router.state.location.pathname).toBe(`/${G}/g-inbox/${second}`);

    // With a thread open, j opens the next one.
    await user.keyboard('j');
    const third = rows()[2].dataset.threadId!;
    await waitFor(() => expect(screen.getByLabelText('open thread')).toHaveTextContent(third));

    await user.keyboard('u');
    await waitFor(() => expect(router.state.location.pathname).toBe(`/${G}/g-inbox`));
    expect(cursorRow()?.dataset.threadId).toBe(third);
  });

  it('selects with x, shift-click ranges and mod-click toggles', async () => {
    const user = userEvent.setup();
    setup();
    await waitFor(() => expect(rows().length).toBeGreaterThan(5));
    const ids = rows().map((r) => r.dataset.threadId);

    await user.keyboard('jx');
    expect(checkedIds()).toEqual([ids[0]]);
    expect(screen.getByText('1 selected')).toBeInTheDocument();

    await user.keyboard('{Shift>}');
    await user.click(within(rows()[3]).getByRole('link'));
    await user.keyboard('{/Shift}');
    expect(checkedIds()).toEqual(ids.slice(0, 4));

    const mod = isMac ? 'Meta' : 'Control';
    await user.keyboard(`{${mod}>}`);
    await user.click(within(rows()[1]).getByRole('link'));
    await user.keyboard(`{/${mod}}`);
    expect(checkedIds()).toEqual([ids[0], ids[2], ids[3]]);

    await user.keyboard('{Escape}');
    expect(checkedIds()).toEqual([]);
  });

  it('selects every row from the header checkbox', async () => {
    const user = userEvent.setup();
    setup();
    await waitFor(() => expect(rows().length).toBeGreaterThan(3));
    await user.click(screen.getByRole('checkbox', { name: 'Select all' }));
    expect(checkedIds()).toHaveLength(rows().length);
    await user.click(screen.getByRole('checkbox', { name: 'Select none' }));
    expect(checkedIds()).toHaveLength(0);
  });

  it('archives the cursor row with e and moves to the next one', async () => {
    const user = userEvent.setup();
    setup();
    await waitFor(() => expect(rows().length).toBeGreaterThan(3));
    const before = rows().map((r) => r.dataset.threadId);

    await user.keyboard('je');
    await waitFor(() => expect(rows().map((r) => r.dataset.threadId)).not.toContain(before[0]));
    expect(cursorRow()?.dataset.threadId).toBe(before[1]);
  });

  it('archives the checked rows from the toolbar', async () => {
    const user = userEvent.setup();
    setup();
    await waitFor(() => expect(rows().length).toBeGreaterThan(3));
    const before = rows().map((r) => r.dataset.threadId);
    await user.click(within(rows()[0]).getByRole('checkbox'));
    await user.click(within(rows()[2]).getByRole('checkbox'));
    await user.click(screen.getByRole('button', { name: 'Archive' }));
    await waitFor(() => {
      const now = rows().map((r) => r.dataset.threadId);
      expect(now).not.toContain(before[0]);
      expect(now).not.toContain(before[2]);
      expect(now).toContain(before[1]);
    });
    expect(screen.queryByText(/selected/)).not.toBeInTheDocument();
  });

  it('stars with s', async () => {
    const user = userEvent.setup();
    setup();
    await waitFor(() => expect(rows().length).toBeGreaterThan(3));
    const row = () => rows()[0];
    const wasStarred =
      within(row()).getByRole('button', { name: /star/i }).getAttribute('aria-pressed') === 'true';
    await user.keyboard('js');
    await waitFor(() =>
      expect(within(row()).getByRole('button', { name: /star/i })).toHaveAttribute(
        'aria-pressed',
        String(!wasStarred),
      ),
    );
  });

  it('merges every account in the unified inbox', async () => {
    setup('/all/inbox');
    await waitFor(() =>
      expect(
        within(screen.getByRole('list', { name: 'All inboxes' })).getAllByRole('listitem').length,
      ).toBeGreaterThan(3),
    );
    const items = within(screen.getByRole('list', { name: 'All inboxes' })).getAllByRole(
      'listitem',
    );
    const links = items.map((li) => within(li).getByRole('link').getAttribute('href') ?? '');
    expect(links.some((h) => h.includes(`account=${MOCK_ACCOUNT_IDS.imap}`))).toBe(true);
    expect(links.some((h) => h.includes(`account=${G}`))).toBe(true);
  });
});
