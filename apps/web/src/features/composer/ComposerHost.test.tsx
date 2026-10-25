import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import { Toaster } from 'sonner';
import {
  EngineFeature,
  ensureConnected,
  EngineProvider,
  MOCK_ACCOUNT_IDS,
  MockEngine,
  type ContactRef,
  type FileJSON,
  type TaskJSON,
} from '@/engine';
import { setPreferences } from '@/lib/preferences';
import { installShortcuts, resetShortcuts } from '@/lib/shortcuts';
import { MOCK_FEATURES } from '@/engine/mock/engine';
import { ComposerHost } from './ComposerHost';
import { formatContact } from './recipients';
import { openComposer, resetComposers } from './store';

const G = MOCK_ACCOUNT_IDS.gmail;
const I = MOCK_ACCOUNT_IDS.imap;

function setup(opts: { features?: readonly string[] } = {}) {
  const engine = new MockEngine({ incomingEveryMs: null, features: opts.features });
  const queued: TaskJSON[] = [];
  const calls: [string, unknown][] = [];
  const call = engine.call.bind(engine);
  vi.spyOn(engine, 'call').mockImplementation(((method: string, params: unknown) => {
    calls.push([method, params]);
    if (method === 'task.queue') queued.push((params as { task: TaskJSON }).task);
    return call(method as never, params as never);
  }) as typeof engine.call);
  // The sends `draft.send` and the expiration executor queue in the engine.
  const sent: TaskJSON[] = [];
  engine.onNotification((n) => {
    if (n.method !== 'delta' || n.params.modelClass !== 'Task') return;
    for (const t of n.params.modelJSONs as TaskJSON[])
      if (t.__cls === 'SendDraftTask' && !sent.some((x) => x.id === t.id)) sent.push(t);
  });
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const uninstall = installShortcuts(window);
  render(
    <QueryClientProvider client={qc}>
      <EngineProvider client={engine}>
        <TooltipProvider>
          <ComposerHost />
          <Toaster />
        </TooltipProvider>
      </EngineProvider>
    </QueryClientProvider>,
  );
  return { engine, queued, calls, sent, uninstall, user: userEvent.setup() };
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

  it('sends with draft.send and closes the window', async () => {
    setPreferences({ undoSendMs: 0 });
    const { calls, sent, user } = setup();
    act(
      () =>
        void openComposer({ mode: 'new', accountId: G, subject: 'Hi', to: [{ email: 'b@x.io' }] }),
    );
    const win = await screen.findByRole('dialog', { name: 'New message' });
    await user.click(await within(win).findByRole('button', { name: /^Send$/ }));
    await waitFor(() => expect(sent).toHaveLength(1));
    expect(calls.map(([m]) => m)).toEqual(
      expect.arrayContaining(['draft.create', 'draft.validate', 'draft.send']),
    );
    const [, params] = calls.find(([m]) => m === 'draft.send')!;
    const [task] = sent;
    expect(params).toEqual({
      accountId: G,
      headerMessageId: task.__cls === 'SendDraftTask' ? task.headerMessageId : null,
    });
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument(), {
      timeout: 3000,
    });
  });

  it('asks before sending without a subject', async () => {
    setPreferences({ undoSendMs: 0 });
    const { sent, user } = setup();
    act(() => void openComposer({ mode: 'new', accountId: G, to: [{ email: 'b@x.io' }] }));
    const win = await screen.findByRole('dialog', { name: 'New message' });
    await user.click(await within(win).findByRole('button', { name: /^Send$/ }));
    expect(await screen.findByText('The subject field is blank.')).toBeInTheDocument();
    expect(sent).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: 'Send anyway' }));
    await waitFor(() => expect(sent).toHaveLength(1));
  });

  describe('undo send on an engine with expiration-executor', () => {
    const features = [...MOCK_FEATURES, EngineFeature.ExpirationExecutor];
    const sendLater = (t: TaskJSON) =>
      t.__cls === 'SyncbackDraftTask'
        ? t.draft.metadata?.find((m) => m.pluginId === 'send-later')?.value
        : undefined;

    async function sendNew(user: ReturnType<typeof userEvent.setup>) {
      act(
        () =>
          void openComposer({
            mode: 'new',
            accountId: G,
            subject: 'Hi',
            to: [{ email: 'b@x.io' }],
          }),
      );
      const win = await screen.findByRole('dialog', { name: 'New message' });
      await user.click(await within(win).findByRole('button', { name: /^Send$/ }));
    }

    it('asks the engine to send after the delay', async () => {
      setPreferences({ undoSendMs: 1000 });
      const { calls, queued, sent, user } = setup({ features });
      await sendNew(user);
      await waitFor(() => expect(calls.some(([m]) => m === 'draft.send')).toBe(true));
      const [, params] = calls.find(([m]) => m === 'draft.send')!;
      expect(params).toMatchObject({ accountId: G, delayMs: 1000 });
      // The client writes no send-later row and queues no send; the engine does both.
      expect(queued.some((t) => sendLater(t))).toBe(false);
      expect(sent).toHaveLength(0);
      await waitFor(() => expect(sent.length).toBeGreaterThan(0), { timeout: 4000 });
      expect(ofClass(queued, 'SendDraftTask')).toHaveLength(0);
      await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument(), {
        timeout: 3000,
      });
    });

    it('Undo cancels the send and nothing is sent', async () => {
      setPreferences({ undoSendMs: 1500 });
      const { calls, sent, user } = setup({ features });
      await sendNew(user);
      // fireEvent: sonner's pointer handlers need setPointerCapture, which jsdom lacks.
      fireEvent.click(await screen.findByRole('button', { name: 'Undo' }));
      await waitFor(() =>
        expect(calls.filter(([m]) => m === 'draft.cancelSend')).toEqual([
          ['draft.cancelSend', { accountId: G, headerMessageId: expect.any(String) }],
        ]),
      );
      expect(await screen.findByRole('dialog', { name: 'New message' })).toBeVisible();
      await new Promise((r) => setTimeout(r, 2500));
      expect(sent).toHaveLength(0);
    });
  });

  it('refuses to send without recipients', async () => {
    const { sent, user } = setup();
    act(() => void openComposer({ mode: 'new', accountId: G, subject: 'Hi' }));
    const win = await screen.findByRole('dialog', { name: 'New message' });
    await user.click(await within(win).findByRole('button', { name: /^Send$/ }));
    expect(
      await screen.findByText(
        'You need to provide one or more recipients before sending the message.',
      ),
    ).toBeInTheDocument();
    expect(sent).toHaveLength(0);
    expect(screen.getByRole('dialog', { name: 'New message' })).toBeVisible();
  });

  it("inserts one of the engine's templates", async () => {
    const { calls, user } = setup();
    act(() => void openComposer({ mode: 'new', accountId: G }));
    const win = await screen.findByRole('dialog', { name: 'New message' });
    await user.click(await within(win).findByRole('button', { name: 'Templates' }));
    await user.click(await screen.findByRole('menuitem', { name: 'Thanks' }));
    await waitFor(() =>
      expect(within(win).getByText('Thanks for getting back to me so quickly.')).toBeVisible(),
    );
    expect((within(win).getByRole('textbox', { name: 'Subject' }) as HTMLInputElement).value).toBe(
      'Thank you',
    );
    expect(calls).toContainEqual(['templates.get', { name: 'Thanks' }]);
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

  it("uploads attachments, then sends them by the engine's id", async () => {
    setPreferences({ undoSendMs: 0 });
    const { engine, sent, user } = setup();
    let finish: (f: FileJSON) => void = () => {};
    const upload = vi
      .spyOn(engine, 'uploadFile')
      .mockImplementation(() => new Promise<FileJSON>((r) => (finish = r)));
    const prepare = vi.spyOn(engine, 'prepareFile');
    act(
      () =>
        void openComposer({ mode: 'new', accountId: G, subject: 'Hi', to: [{ email: 'b@x.io' }] }),
    );
    const win = await screen.findByRole('dialog', { name: 'New message' });
    await within(win).findByRole('button', { name: 'Attach files' });
    const input = win.querySelector<HTMLInputElement>('input[type=file]')!;
    const picked = new File(['hello'], 'notes.txt', { type: 'text/plain' });
    fireEvent.change(input, { target: { files: [picked] } });
    expect(upload).toHaveBeenCalledWith(G, picked, 'notes.txt');
    expect(await within(win).findByLabelText('Uploading')).toBeInTheDocument();

    // Not while the upload runs.
    await user.click(within(win).getByRole('button', { name: /^Send$/ }));
    expect(sent).toHaveLength(0);

    const stored: FileJSON = {
      id: 'engine-file-1',
      aid: G,
      v: 1,
      __cls: 'File',
      messageId: '',
      contentId: null,
      contentType: 'text/plain',
      filename: 'notes.txt',
      size: 5,
    };
    act(() => finish(stored));
    await waitFor(() => expect(within(win).queryByLabelText('Uploading')).toBeNull());
    await user.click(within(win).getByRole('button', { name: /^Send$/ }));
    await waitFor(() => expect(sent).toHaveLength(1));
    const [send] = sent;
    expect(send.__cls === 'SendDraftTask' && send.draft.files.map((f) => f.id)).toEqual([
      'engine-file-1',
    ]);
    expect(prepare).toHaveBeenCalledWith(expect.objectContaining({ id: 'engine-file-1' }));
  });

  it('uploads picked files again when From moves to another account', async () => {
    setPreferences({ undoSendMs: 0 });
    const { engine, queued, sent, user } = setup();
    const upload = vi.spyOn(engine, 'uploadFile');
    act(
      () =>
        void openComposer({ mode: 'new', accountId: G, subject: 'Hi', to: [{ email: 'b@x.io' }] }),
    );
    let win = await screen.findByRole('dialog', { name: 'New message' });
    await within(win).findByRole('button', { name: 'Attach files' });
    const picked = new File(['hello'], 'notes.txt', { type: 'text/plain' });
    fireEvent.change(win.querySelector<HTMLInputElement>('input[type=file]')!, {
      target: { files: [picked] },
    });
    await waitFor(() => expect(within(win).queryByLabelText('Uploading')).toBeNull());

    const from = within(win).getByRole('combobox', { name: 'From' });
    from.focus();
    await user.keyboard('{Enter}');
    await user.click(await screen.findByRole('option', { name: /maya@okafor\.studio/ }));
    await waitFor(() => expect(upload).toHaveBeenCalledWith(I, picked, 'notes.txt'));
    const moved = await upload.mock.results[1].value;
    await waitFor(() => expect(within(win).queryByLabelText('Uploading')).toBeNull());

    // Saved under the new account, the draft names the File the engine keeps there.
    await waitFor(
      () =>
        expect(
          ofClass(queued, 'SyncbackDraftTask').filter(
            (t) => t.__cls === 'SyncbackDraftTask' && t.draft.aid === I,
          ),
        ).not.toHaveLength(0),
      { timeout: 3000 },
    );
    const saved = ofClass(queued, 'SyncbackDraftTask').at(-1)!;
    const draft = saved.__cls === 'SyncbackDraftTask' ? saved.draft : null;
    expect(draft!.files.map((f) => [f.id, f.aid])).toEqual([[moved.id, I]]);

    // Reopened and sent, the attachment is still there to fetch.
    await user.click(within(win).getByRole('button', { name: 'Close and keep draft' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    act(() => void openComposer({ mode: 'edit-draft', accountId: I, draftId: draft!.id }));
    win = await screen.findByRole('dialog');
    await within(win).findByText('notes.txt');
    await user.click(within(win).getByRole('button', { name: /^Send$/ }));
    await waitFor(() => expect(sent).toHaveLength(1));
    const [send] = sent;
    expect(send.__cls === 'SendDraftTask' && send.draft.files.map((f) => f.id)).toEqual([moved.id]);
  });

  it("doesn't send when an attachment can't be had", async () => {
    setPreferences({ undoSendMs: 0 });
    const { engine, sent, user } = setup();
    vi.spyOn(engine, 'prepareFile').mockRejectedValue(new Error('gone'));
    act(
      () =>
        void openComposer({ mode: 'new', accountId: G, subject: 'Hi', to: [{ email: 'b@x.io' }] }),
    );
    const win = await screen.findByRole('dialog', { name: 'New message' });
    await within(win).findByRole('button', { name: 'Attach files' });
    const input = win.querySelector<HTMLInputElement>('input[type=file]')!;
    fireEvent.change(input, { target: { files: [new File(['x'], 'a.txt')] } });
    await waitFor(() => expect(within(win).queryByLabelText('Uploading')).toBeNull());
    await user.click(within(win).getByRole('button', { name: /^Send$/ }));
    await waitFor(() => expect(screen.getByRole('dialog', { name: 'New message' })).toBeVisible());
    await new Promise((r) => setTimeout(r, 50));
    expect(sent).toHaveLength(0);
  });
});
