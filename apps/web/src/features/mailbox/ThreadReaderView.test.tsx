import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from '@tanstack/react-router';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import {
  EngineProvider,
  MOCK_ACCOUNT_IDS,
  MockEngine,
  type EngineNotification,
  type TaskJSON,
  type ThreadJSON,
} from '@/engine';
import { useComposerWindows } from '@/features/composer';
import { installShortcuts, resetShortcuts } from '@/lib/shortcuts';
import { ThreadReaderView } from './ThreadReaderView';

const G = MOCK_ACCOUNT_IDS.gmail;

function Composers() {
  const windows = useComposerWindows();
  return <output aria-label="composers">{windows.map((w) => w.mode).join(',')}</output>;
}

async function setup(pick: (t: ThreadJSON) => boolean) {
  const engine = new MockEngine({ latency: 0 });
  await engine.connect();
  const { threads: items } = await engine.call('threads.list', {
    accountId: G,
    folderId: 'g-all',
    limit: 200,
  });
  const target = items.find(pick);
  if (!target) throw new Error('no fixture thread matches');
  const queued: TaskJSON[] = [];
  engine.onNotification((n: EngineNotification) => {
    if (n.method === 'delta' && n.params.modelClass === 'Task') {
      queued.push(...(n.params.modelJSONs as TaskJSON[]));
    }
  });

  const root = createRootRoute();
  const thread = createRoute({
    getParentRoute: () => root,
    path: '$accountId/$folderId/$threadId',
    component: function Thread() {
      const { accountId, threadId } = thread.useParams();
      return (
        <>
          <ThreadReaderView accountId={accountId} threadId={threadId} />
          <Composers />
        </>
      );
    },
  });
  const router = createRouter({
    routeTree: root.addChildren([thread]),
    history: createMemoryHistory({ initialEntries: [`/${G}/g-inbox/${target.id}`] }),
  });
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <EngineProvider client={engine}>
        <TooltipProvider>
          <RouterProvider router={router} />
        </TooltipProvider>
      </EngineProvider>
    </QueryClientProvider>,
  );
  return { target, queued };
}

const messageCount = (t: ThreadJSON) =>
  Math.max(...[...t.folders, ...t.labels].map((r) => r._refs));

let uninstall: () => void;
beforeEach(() => {
  window.scrollTo = () => {};
  uninstall = installShortcuts(window);
});
afterEach(() => {
  uninstall();
  resetShortcuts();
});

describe('ThreadReaderView', () => {
  it('marks an unread thread read on open', async () => {
    const { target, queued } = await setup((t) => t.unread > 0);
    await screen.findByRole('heading', { name: target.subject });
    await waitFor(() =>
      expect(
        queued.some(
          (t) => t.__cls === 'ChangeUnreadTask' && !t.unread && t.threadIds.includes(target.id),
        ),
      ).toBe(true),
    );
  });

  it('collapses read messages except the last and expands on click', async () => {
    const user = userEvent.setup();
    await setup((t) => t.unread === 0 && messageCount(t) >= 2);
    const articles = await screen.findAllByRole('article');
    expect(articles).toHaveLength(1);
    const collapsed = screen.getAllByRole('button', { expanded: false });
    await user.click(collapsed[0]);
    expect(screen.getAllByRole('article')).toHaveLength(2);
  });

  it('replies to the last message with r', async () => {
    const user = userEvent.setup();
    await setup((t) => t.unread === 0 && !t.labels.some((l) => l.role === 'drafts'));
    await screen.findAllByRole('article');
    await user.keyboard('r');
    expect(screen.getByLabelText('composers')).toHaveTextContent('reply');
  });
});
