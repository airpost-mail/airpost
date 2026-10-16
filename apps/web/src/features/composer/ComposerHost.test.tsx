import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import {
  ensureConnected,
  EngineProvider,
  MOCK_ACCOUNT_IDS,
  MockEngine,
  type ContactRef,
  type TaskJSON,
} from '@/engine';
import { setPreferences } from '@/lib/preferences';
import { installShortcuts, resetShortcuts } from '@/lib/shortcuts';
import { ComposerHost } from './ComposerHost';
import { formatContact } from './recipients';
import { openComposer, resetComposers } from './store';

const G = MOCK_ACCOUNT_IDS.gmail;

function setup() {
  const engine = new MockEngine({ incomingEveryMs: null });
  const queued: TaskJSON[] = [];
  const call = engine.call.bind(engine);
  vi.spyOn(engine, 'call').mockImplementation(((method: string, params: unknown) => {
    if (method === 'task.queue') queued.push((params as { task: TaskJSON }).task);
    return call(method as never, params as never);
  }) as typeof engine.call);
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const uninstall = installShortcuts(window);
  render(
    <QueryClientProvider client={qc}>
      <EngineProvider client={engine}>
        <TooltipProvider>
          <ComposerHost />
        </TooltipProvider>
      </EngineProvider>
    </QueryClientProvider>,
  );
  return { engine, queued, uninstall, user: userEvent.setup() };
}

/** Waits for the window's initial focus so typing is not redirected mid-way. */
async function focused<T extends HTMLElement>(el: Promise<T>): Promise<T> {
  const node = await el;
  await waitFor(() => expect(node).toHaveFocus());
  return node;
}

const ofClass = (queued: TaskJSON[], cls: TaskJSON['__cls']) =>
  queued.filter((t) => t.__cls === cls);

describe('ComposerHost', () => {
  let cleanupShortcuts: () => void = () => {};
  beforeEach(() => {
    resetComposers();
    resetShortcuts();
  });
  afterEach(() => {
    cleanupShortcuts();
    act(() => resetComposers());
  });

  it('opens a window with c and autosaves after edits settle', async () => {
    const { queued, uninstall, user } = setup();
    cleanupShortcuts = uninstall;
    await user.keyboard('c');
    const win = await screen.findByRole('dialog', { name: 'New message' });
    const to = await focused(within(win).findByRole('combobox', { name: 'To' }));
    await user.type(to, 'Ann Lee <ann@example.com>, ');
    expect(within(win).getByText('Ann Lee')).toBeInTheDocument();
    await user.type(within(win).getByRole('textbox', { name: 'Subject' }), 'Hello');

    expect(ofClass(queued, 'SyncbackDraftTask')).toHaveLength(0);
    await waitFor(() => expect(ofClass(queued, 'SyncbackDraftTask')).toHaveLength(1), {
      timeout: 3000,
    });
    const [save] = ofClass(queued, 'SyncbackDraftTask');
    expect(save.__cls === 'SyncbackDraftTask' && save.draft).toMatchObject({
      aid: G,
      subject: 'Hello',
      draft: true,
      to: [{ name: 'Ann Lee', email: 'ann@example.com' }],
    });
  });

  it('builds a reply to the last message of a thread', async () => {
    const { engine } = setup();
    await ensureConnected(engine);
    const page = await engine.call('threads.list', { accountId: G, limit: 20 });
    let target: { threadId: string; sender: ContactRef } | undefined;
    for (const t of page.threads) {
      const { messages } = await engine.call('threads.get', { accountId: G, id: t.id });
      const last = messages
        .filter((m) => !m.draft)
        .sort((a, b) => a.date - b.date)
        .at(-1);
      const sender = last?.from[0];
      if (sender && !sender.email.startsWith('maya') && last.replyTo.length === 0) {
        target = { threadId: t.id, sender };
        break;
      }
    }
    expect(target).toBeDefined();

    act(() => void openComposer({ mode: 'reply', accountId: G, threadId: target!.threadId }));
    const win = await screen.findByRole('dialog', { name: 'Reply' });
    const subject = await within(win).findByRole('textbox', { name: 'Subject' });
    expect((subject as HTMLInputElement).value).toMatch(/^Re: /);
    expect(within(win).getByTitle(formatContact(target!.sender))).toBeInTheDocument();
  });

  it('sends with SendDraftTask and closes the window', async () => {
    setPreferences({ undoSendMs: 0 });
    const { queued, user } = setup();
    act(
      () =>
        void openComposer({ mode: 'new', accountId: G, subject: 'Hi', to: [{ email: 'b@x.io' }] }),
    );
    const win = await screen.findByRole('dialog', { name: 'New message' });
    await user.click(await within(win).findByRole('button', { name: /^Send$/ }));
    await waitFor(() => expect(ofClass(queued, 'SendDraftTask')).toHaveLength(1));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument(), {
      timeout: 3000,
    });
  });

  it('refuses to send without recipients', async () => {
    const { queued, user } = setup();
    act(() => void openComposer({ mode: 'new', accountId: G, subject: 'Hi' }));
    const win = await screen.findByRole('dialog', { name: 'New message' });
    await user.click(await within(win).findByRole('button', { name: /^Send$/ }));
    expect(ofClass(queued, 'SendDraftTask')).toHaveLength(0);
    expect(screen.getByRole('dialog', { name: 'New message' })).toBeVisible();
  });

  it('discards a saved draft with DestroyDraftTask', async () => {
    const { queued, user } = setup();
    act(() => void openComposer({ mode: 'new', accountId: G }));
    const win = await screen.findByRole('dialog', { name: 'New message' });
    await focused(within(win).findByRole('combobox', { name: 'To' }));
    await user.type(within(win).getByRole('textbox', { name: 'Subject' }), 'Plans');
    await waitFor(() => expect(ofClass(queued, 'SyncbackDraftTask')).toHaveLength(1), {
      timeout: 3000,
    });
    await user.click(within(win).getByRole('button', { name: 'Discard draft' }));
    await waitFor(() => expect(ofClass(queued, 'DestroyDraftTask')).toHaveLength(1));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('closing an untouched draft saves nothing', async () => {
    const { queued, user } = setup();
    act(() => void openComposer({ mode: 'new', accountId: G }));
    const win = await screen.findByRole('dialog', { name: 'New message' });
    await user.click(await within(win).findByRole('button', { name: 'Close and keep draft' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(queued).toHaveLength(0);
  });
});
