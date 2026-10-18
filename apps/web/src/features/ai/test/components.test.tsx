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
import { useRef, type ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import {
  EngineProvider,
  MOCK_ACCOUNT_IDS,
  MockEngine,
  resetEngineState,
  threadHasRole,
  type MessageJSON,
  type RpcMethodName,
  type ThreadJSON,
} from '@/engine';
import { BodyEditor, type BodyEditorHandle } from '@/features/composer/BodyEditor';
import { AiSettingsView } from '../AiSettings';
import { NeedsReplyView } from '../NeedsReplyView';
import { SuggestReply } from '../SuggestReply';
import { forgetSummaries } from '../summary-memory';
import { ThreadAi } from '../ThreadSummary';
import { triageOf } from '../triage';
import { TriageChip } from '../TriageChip';

const G = MOCK_ACCOUNT_IDS.gmail;

afterEach(() => {
  forgetSummaries();
  resetEngineState();
});

async function setup(opts: { aiDemo?: boolean; latency?: number } = {}) {
  const engine = new MockEngine({ latency: opts.latency ?? 0, aiDemo: opts.aiDemo ?? true });
  await engine.connect();
  const calls: { method: RpcMethodName; params: unknown }[] = [];
  const real = engine.call.bind(engine);
  vi.spyOn(engine, 'call').mockImplementation(((method: RpcMethodName, params: unknown) => {
    calls.push({ method, params });
    return (real as (m: RpcMethodName, p: unknown) => Promise<unknown>)(method, params);
  }) as typeof engine.call);
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrap = (ui: ReactNode) => (
    <QueryClientProvider client={qc}>
      <EngineProvider client={engine}>
        <TooltipProvider>{ui}</TooltipProvider>
      </EngineProvider>
    </QueryClientProvider>
  );
  const of = (m: RpcMethodName) => calls.filter((c) => c.method === m);
  return { engine, qc, wrap, calls, of };
}

function conversation(engine: MockEngine): { thread: ThreadJSON; messages: MessageJSON[] } {
  for (const t of engine.store.threads.values()) {
    const messages = engine.store.threadMessages(t.id).filter((m) => !m.draft);
    if (t.aid === G && messages.length >= 2) return { thread: t, messages };
  }
  throw new Error('no fixture thread');
}

describe('ThreadAi', () => {
  it('streams a summary into the card as plain text and remembers it', async () => {
    const t = await setup();
    const { thread, messages } = conversation(t.engine);
    const user = userEvent.setup();
    const view = render(t.wrap(<ThreadAi thread={thread} messages={messages} />));
    expect(await screen.findByText("Runs on the engine's computer (Ollama)")).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Summarize' }));
    const card = await screen.findByRole('region', { name: 'Summary' });
    await waitFor(() => expect(within(card).getByText(/wrote \d+ messages/)).toBeInTheDocument());
    expect(within(card).getByText(/llama3\.2/)).toBeInTheDocument();
    expect(t.of('ai.summarize')[0].params).toMatchObject({
      accountId: G,
      threadId: thread.id,
      refresh: false,
    });
    expect(t.of('ai.summarize')[0].params).not.toHaveProperty('config');

    view.unmount();
    render(t.wrap(<ThreadAi thread={thread} messages={messages} />));
    expect(await screen.findByRole('region', { name: 'Summary' })).toBeInTheDocument();
    expect(t.of('ai.summarize')).toHaveLength(1);
  });

  it('shows model output without HTML or links', async () => {
    const t = await setup();
    const { thread, messages } = conversation(t.engine);
    const call = vi.mocked(t.engine.call);
    const real = call.getMockImplementation()!;
    call.mockImplementation(((m: RpcMethodName, p: unknown) =>
      m === 'ai.summarize'
        ? Promise.resolve({ text: 'Click <a href="https://evil.example">here</a>', model: 'm' })
        : (real as (m: RpcMethodName, p: unknown) => Promise<unknown>)(
            m,
            p,
          )) as typeof t.engine.call);
    const user = userEvent.setup();
    render(t.wrap(<ThreadAi thread={thread} messages={messages} />));
    await user.click(await screen.findByRole('button', { name: 'Summarize' }));
    const card = await screen.findByRole('region', { name: 'Summary' });
    expect(await within(card).findByText(/Click <a href/)).toBeInTheDocument();
    expect(card.querySelector('a')).toBeNull();
  });

  it('cancels the call when the card closes', async () => {
    const t = await setup({ latency: 20 });
    const { thread, messages } = conversation(t.engine);
    const user = userEvent.setup();
    render(t.wrap(<ThreadAi thread={thread} messages={messages} />));
    await user.click(await screen.findByRole('button', { name: 'Summarize' }));
    await user.click(await screen.findByRole('button', { name: 'Close summary' }));
    await waitFor(() => expect(t.of('ai.cancel')).toHaveLength(1));
    const { streamId } = t.of('ai.summarize')[0].params as { streamId: string };
    expect(t.of('ai.cancel')[0].params).toEqual({ streamId });
    expect(screen.getByRole('button', { name: 'Summarize' })).toBeInTheDocument();
  });

  it('stays out of the way when AI is off, and says why a key is missing', async () => {
    const t = await setup({ aiDemo: false });
    const { thread, messages } = conversation(t.engine);
    render(t.wrap(<ThreadAi thread={thread} messages={messages} />));
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(screen.queryByRole('button', { name: 'Summarize' })).toBeNull();

    act(() =>
      t.engine.changeSettings({
        'core.ai.providers': [{ id: 'c', kind: 'anthropic', model: 'claude' }],
        'core.ai.features': { summary: { enabled: true, provider: 'c' } },
        'core.ai.accounts': [G],
      }),
    );
    const button = await screen.findByRole('button', { name: 'Summarize' });
    expect(button).toBeDisabled();
    expect(screen.getByText(/doesn't hold this provider's API key/)).toBeInTheDocument();
  });
});

function Composer({ accountId, threadId }: { accountId: string; threadId: string }) {
  const editor = useRef<BodyEditorHandle>(null);
  return (
    <>
      <BodyEditor ref={editor} initialHtml="" onChange={() => {}} />
      <SuggestReply accountId={accountId} threadId={threadId} editor={editor} />
    </>
  );
}

describe('SuggestReply', () => {
  it('sends nothing until asked, then writes the picked reply into the body', async () => {
    const t = await setup();
    const { thread } = conversation(t.engine);
    const user = userEvent.setup();
    render(t.wrap(<Composer accountId={G} threadId={thread.id} />));
    await user.click(await screen.findByRole('button', { name: 'Suggest a reply' }));
    expect(t.of('ai.suggestReplies')).toHaveLength(0);
    await user.click(screen.getByRole('button', { name: 'Get suggestions' }));
    const list = await screen.findByRole('list', { name: 'Suggestions' });
    const picks = within(list).getAllByRole('button');
    expect(picks).toHaveLength(3);
    await user.click(picks[1]);
    const body = screen.getByRole('textbox', { name: 'Message body' });
    await waitFor(() => expect(body.textContent).toContain('Say yes and confirm the details.'));
    expect(t.of('ai.draftReply')[0].params).toMatchObject({
      suggestion: 'Say yes and confirm the details.',
    });
    expect(screen.queryByRole('list', { name: 'Suggestions' })).toBeNull();
  });

  it('writes from an instruction', async () => {
    const t = await setup();
    const { thread } = conversation(t.engine);
    const user = userEvent.setup();
    render(t.wrap(<Composer accountId={G} threadId={thread.id} />));
    await user.click(await screen.findByRole('button', { name: 'Suggest a reply' }));
    await user.type(screen.getByRole('textbox', { name: 'What to say' }), "say I'll call Tuesday");
    await user.click(screen.getByRole('button', { name: 'Write' }));
    const body = screen.getByRole('textbox', { name: 'Message body' });
    await waitFor(() => expect(body.textContent).toContain("Say I'll call Tuesday."));
  });
});

describe('TriageChip', () => {
  it('shows needs-reply and files a correction as the user', async () => {
    const t = await setup();
    const thread = [...t.engine.store.threads.values()].find(
      (x) => triageOf(x)?.category === 'needs-reply' && !x.lmst,
    )!;
    const user = userEvent.setup();
    const view = render(t.wrap(<TriageChip thread={thread} />));
    await user.click(await screen.findByRole('button', { name: 'Needs reply: change category' }));
    await user.click(await screen.findByRole('menuitemradio', { name: 'Important' }));
    await waitFor(() => expect(t.of('task.queue')).toHaveLength(1));
    const task = (t.of('task.queue')[0].params as { task: { value: object } }).task;
    expect(task.value).toMatchObject({ category: 'important', source: 'user' });

    const after = t.engine.store.threads.get(thread.id)!;
    view.rerender(t.wrap(<TriageChip thread={after} />));
    expect(screen.getByRole('button', { name: 'Important: change category' })).toBeInTheDocument();
  });

  it('shows nothing for an account without triage', async () => {
    const t = await setup();
    const thread = [...t.engine.store.threads.values()].find(
      (x) => triageOf(x)?.category === 'needs-reply' && !x.lmst,
    )!;
    act(() => t.engine.changeSettings({ 'core.ai.accounts': [] }));
    render(t.wrap(<TriageChip thread={thread} />));
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(screen.queryByRole('button', { name: /change category/ })).toBeNull();
  });
});

function routed(wrap: (ui: ReactNode) => ReactNode, path: string) {
  const root = createRootRoute();
  const page = createRoute({
    getParentRoute: () => root,
    path: 'needs-reply',
    validateSearch: (s: Record<string, unknown>) => s as { account?: string; thread?: string },
    component: function Page() {
      const s = page.useSearch() as { account?: string; thread?: string; threadAccount?: string };
      const open =
        s.thread && s.threadAccount ? { accountId: s.threadAccount, threadId: s.thread } : null;
      return (
        <NeedsReplyView
          accountId={s.account}
          openThread={open}
          reader={<output aria-label="reader">{open?.threadId ?? 'none'}</output>}
        />
      );
    },
  });
  const router = createRouter({
    routeTree: root.addChildren([page]),
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  render(wrap(<RouterProvider router={router} />));
  return router;
}

describe('NeedsReplyView', () => {
  it('lists the triaged threads and opens one in the reader', async () => {
    const t = await setup();
    routed(t.wrap, '/needs-reply');
    const list = await screen.findByRole('list', { name: 'Needs reply' });
    const expected = await t.engine.call('ai.triage.list', {
      category: 'needs-reply',
      limit: 500,
    });
    await waitFor(() =>
      expect(within(list).getAllByRole('listitem')).toHaveLength(expected.threadIds.length),
    );
    const user = userEvent.setup();
    await user.click(within(list).getAllByRole('link')[0]);
    await waitFor(() =>
      expect(screen.getByLabelText('reader')).toHaveTextContent(expected.threadIds[0]),
    );
  });

  it('filters by account and refreshes when triage files a thread', async () => {
    const t = await setup();
    routed(t.wrap, `/needs-reply?account=${G}`);
    const list = await screen.findByRole('list', { name: 'Needs reply' });
    const before = within(list).getAllByRole('listitem').length;
    expect(
      t
        .of('ai.triage.list')
        .every((c) => (c.params as { accountIds: string[] }).accountIds[0] === G),
    ).toBe(true);

    const fresh = [...t.engine.store.threads.values()].find(
      (x) => x.aid === G && threadHasRole(x, 'inbox') && triageOf(x)?.category !== 'needs-reply',
    )!;
    await act(async () => {
      await t.engine.call('task.queue', {
        accountId: G,
        task: (await import('../triage')).correctionTask(fresh, 'needs-reply'),
      });
    });
    await waitFor(() => expect(within(list).getAllByRole('listitem')).toHaveLength(before + 1), {
      timeout: 3000,
    });
  });

  it('says when triage is off', async () => {
    const t = await setup({ aiDemo: false });
    routed(t.wrap, '/needs-reply');
    expect(await screen.findByText('No triage here')).toBeInTheDocument();
  });
});

describe('AiSettingsView', () => {
  it('shows the engine settings read-only and checks a provider', async () => {
    const t = await setup();
    await t.engine.call('ai.summarize', { ...idsOf(t.engine) });
    const user = userEvent.setup();
    render(t.wrap(<AiSettingsView />));
    const features = await screen.findByRole('region', { name: 'Features' });
    await waitFor(() => expect(within(features).getAllByText('On')).toHaveLength(3));
    const providers = screen.getByRole('region', { name: 'Providers' });
    expect(within(providers).getByText(/llama3\.2/)).toBeInTheDocument();
    await user.click(within(providers).getByRole('button', { name: 'Check' }));
    expect(await within(providers).findByText('Answered in 42 ms (llama3.2).')).toBeInTheDocument();
    expect(t.of('ai.test')[0].params).toEqual({ provider: 'ollama' });
    const usage = screen.getByRole('region', { name: 'Last 30 days' });
    expect(await within(usage).findByText(/1 requests/)).toBeInTheDocument();
    expect(screen.queryByRole('switch')).toBeNull();
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(t.of('settings.set')).toHaveLength(0);
  });
});

function idsOf(engine: MockEngine) {
  const { thread } = conversation(engine);
  return { accountId: thread.aid, threadId: thread.id };
}
