import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TooltipProvider } from '@/components/ui/tooltip';
import {
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
import { ComposerHost } from './ComposerHost';
import { formatContact } from './recipients';
import { openComposer, resetComposers } from './store';

const G = MOCK_ACCOUNT_IDS.gmail;
const I = MOCK_ACCOUNT_IDS.imap;

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

  it("uploads attachments, then sends them by the engine's id", async () => {
    setPreferences({ undoSendMs: 0 });
    const { engine, queued, user } = setup();
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
    expect(ofClass(queued, 'SendDraftTask')).toHaveLength(0);

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
    await waitFor(() => expect(ofClass(queued, 'SendDraftTask')).toHaveLength(1));
    const [send] = ofClass(queued, 'SendDraftTask');
    expect(send.__cls === 'SendDraftTask' && send.draft.files.map((f) => f.id)).toEqual([
      'engine-file-1',
    ]);
    expect(prepare).toHaveBeenCalledWith(expect.objectContaining({ id: 'engine-file-1' }));
  });

  it('uploads picked files again when From moves to another account', async () => {
    setPreferences({ undoSendMs: 0 });
    const { engine, queued, user } = setup();
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
    await waitFor(() => expect(ofClass(queued, 'SendDraftTask')).toHaveLength(1));
    const [send] = ofClass(queued, 'SendDraftTask');
    expect(send.__cls === 'SendDraftTask' && send.draft.files.map((f) => f.id)).toEqual([moved.id]);
  });

  it("doesn't send when an attachment can't be had", async () => {
    setPreferences({ undoSendMs: 0 });
    const { engine, queued, user } = setup();
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
    expect(ofClass(queued, 'SendDraftTask')).toHaveLength(0);
  });
});
