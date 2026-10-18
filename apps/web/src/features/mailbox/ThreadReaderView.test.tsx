import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from '@tanstack/react-router';
import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import {
  EngineProvider,
  MOCK_ACCOUNT_IDS,
  MockEngine,
  type EngineNotification,
  type TaskJSON,
  tasks,
  type ThreadJSON,
} from '@/engine';
import { useComposerWindows } from '@/features/composer';
import { resetComposers } from '@/features/composer/store';
import { installShortcuts, resetShortcuts } from '@/lib/shortcuts';
import { ThreadReaderView } from './ThreadReaderView';

const G = MOCK_ACCOUNT_IDS.gmail;

function Composers() {
  const windows = useComposerWindows();
  return (
    <output aria-label="composers">
      {windows.map((w) => `${w.mode}:${w.placement}`).join(',')}
    </output>
  );
}

async function setup(
  pick: (t: ThreadJSON) => boolean,
  before?: (engine: MockEngine, target: ThreadJSON) => Promise<void>,
  opts: { features?: string[] } = {},
) {
  const engine = new MockEngine({ latency: 0, features: opts.features });
  await engine.connect();
  const { threads: items } = await engine.call('threads.list', {
    accountId: G,
    folderId: 'g-all',
    limit: 200,
  });
  const target = items.find(pick);
  if (!target) throw new Error('no fixture thread matches');
  await before?.(engine, target);
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
  return { target, queued, engine };
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
  act(() => resetComposers());
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
    // Menu triggers (Snooze) are collapsed buttons too.
    const collapsed = screen
      .getAllByRole('button', { expanded: false })
      .filter((b) => !b.hasAttribute('aria-haspopup'));
    await user.click(collapsed[0]);
    expect(screen.getAllByRole('article')).toHaveLength(2);
  });

  it('replies to the last message with r', async () => {
    const user = userEvent.setup();
    await setup((t) => t.unread === 0 && !t.labels.some((l) => l.role === 'drafts'));
    await screen.findAllByRole('article');
    await user.keyboard('r');
    expect(screen.getByLabelText('composers')).toHaveTextContent('reply:inline');
    const reply = await screen.findByRole('region', { name: 'Reply' });
    expect(await within(reply).findByRole('textbox', { name: 'Message body' })).toBeVisible();
  });

  it('reopens a reply draft inline when its thread opens', async () => {
    await setup(
      (t) => t.unread === 0 && messageCount(t) >= 2,
      async (engine, target) => {
        const { messages } = await engine.call('threads.get', { accountId: G, id: target.id });
        const last = messages.at(-1)!;
        const draft = {
          ...last,
          id: 'draft-inline-test',
          hMsgId: 'draft-inline-test@tern',
          rthMsgId: last.hMsgId,
          draft: true,
          date: last.date + 60,
          body: '<p>Half-written reply</p>',
          plaintext: false,
        };
        await engine.call('task.queue', { accountId: G, task: tasks.syncbackDraft(draft) });
      },
    );
    await waitFor(() =>
      expect(screen.getByLabelText('composers')).toHaveTextContent('edit-draft:inline'),
    );
    const region = await screen.findByRole('region', { name: 'Draft' });
    expect(await within(region).findByText('Half-written reply')).toBeInTheDocument();
    // Shown as the composer, not also as a message in the list above it.
    expect(screen.queryAllByText('Half-written reply')).toHaveLength(1);
  });

  const inInbox = (t: ThreadJSON) => t.unread === 0 && t.labels.some((l) => l.role === 'inbox');

  it('snoozes through the engine from the menu', async () => {
    const user = userEvent.setup();
    const { target, queued } = await setup(inInbox);
    await screen.findAllByRole('article');
    await user.click(screen.getByRole('button', { name: 'Snooze' }));
    await user.click(await screen.findByRole('menuitem', { name: /^Tomorrow/ }));
    await waitFor(() =>
      expect(
        queued.some(
          (t) =>
            t.__cls === 'ChangeLabelsTask' &&
            t.threadIds.includes(target.id) &&
            t.labelsToAdd.some((l) => l.role === 'snoozed'),
        ),
      ).toBe(true),
    );
    const meta = queued.find((t) => t.__cls === 'SyncbackMetadataTask' && t.modelId === target.id);
    expect(meta).toMatchObject({ pluginId: 'thread-snooze', modelClassName: 'thread' });
  });

  it('has no snooze or unsubscribe on an engine without them', async () => {
    await setup(
      inInbox,
      async (engine, target) => {
        const last = engine.store.threadMessages(target.id).at(-1)!;
        last.hListUnsub = '<https://lists.example/u>';
      },
      { features: ['query', 'files'] },
    );
    await screen.findAllByRole('article');
    expect(screen.queryByRole('button', { name: 'Snooze' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Unsubscribe' })).toBeNull();
  });

  it('unsubscribes through the engine', async () => {
    const user = userEvent.setup();
    const { engine, target } = await setup(inInbox, async (engine, target) => {
      const last = engine.store.threadMessages(target.id).at(-1)!;
      last.hListUnsub = '<https://lists.example/u/9>';
      last.hListUnsubPost = 'List-Unsubscribe=One-Click';
    });
    const call = vi.spyOn(engine, 'call');
    await user.click(await screen.findByRole('button', { name: 'Unsubscribe' }));
    const last = engine.store.threadMessages(target.id).at(-1)!;
    await waitFor(() =>
      expect(call).toHaveBeenCalledWith('messages.unsubscribe', {
        accountId: G,
        messageId: last.id,
      }),
    );
    const r = call.mock.results[call.mock.calls.findIndex((c) => c[0] === 'messages.unsubscribe')];
    await expect(r!.value).resolves.toEqual({
      method: 'one-click',
      ok: true,
      url: 'https://lists.example/u/9',
    });
  });
});
