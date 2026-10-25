// The state and actions of one composer window: edits, autosave, send with undo, schedule,
// discard. UI components only call into this.
import { useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { generateId, tasks, useEngine, type ContactRef } from '@/engine';
import { getUndoSendMs } from '@/lib/undo-send';
import type { AutosaveStatus } from './autosave';
import {
  allRecipients,
  changeIdentity,
  draftToMessage,
  setMetadata,
  type DraftAttachment,
  type DraftState,
} from './draft';
import { DraftSaver } from './draft-saver';
import { queueTask, watchEngineSend } from './engine-calls';
import { isBlankHtml } from './html';
import type { Identity } from './identities';
import { dedupeContacts, type ParsedRecipients } from './recipients';
import { formatSendLater, SEND_LATER_PLUGIN_ID } from './send-later';
import { withoutSignature } from './signature';
import { closeComposer, focusComposer, setComposerEditingId, setComposerHidden } from './store';

export type RecipientKind = 'to' | 'cc' | 'bcc';

export interface PendingRecipients {
  /** What each recipient field still had typed, parsed. */
  parsed: Partial<Record<RecipientKind, ParsedRecipients>>;
  onNoRecipients?: () => void;
}

const NO_PENDING: PendingRecipients = { parsed: {} };

export interface RecipientFieldHandle {
  /** Turns any typed text into chips; returns what it could not parse. */
  commit(): ParsedRecipients;
  focus(): void;
}

/** Most providers reject messages above 25 MB once encoded. */
const ATTACHMENT_WARN_BYTES = 25 * 1024 * 1024;

/** How long past the undo delay to follow the engine's send before handing it off. */
const ENGINE_SEND_WAIT_MS = 60_000;

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function useComposerSession(args: {
  windowKey: string;
  initial: { draft: DraftState; existing: boolean };
}) {
  const { windowKey, initial } = args;
  const client = useEngine();
  const [draft, setDraft] = useState(initial.draft);
  // For a send the user confirms from a toast, which outlives the render that showed it.
  const latest = useRef(draft);
  useEffect(() => {
    latest.current = draft;
  });
  const [status, setStatus] = useState<AutosaveStatus>(initial.existing ? 'saved' : 'idle');
  const [busy, setBusy] = useState(false);
  const dirty = useRef(false);
  // Set while a send or schedule owns the draft: a save queued after SendDraftTask would
  // bring the sent message back as a draft.
  const handedOff = useRef(false);

  const [saver] = useState(
    () =>
      new DraftSaver(client, {
        existing: initial.existing
          ? { id: initial.draft.id, accountId: initial.draft.accountId }
          : null,
        onStatus: setStatus,
      }),
  );
  const { autosave } = saver;

  useEffect(() => setComposerEditingId(windowKey, draft.id), [windowKey, draft.id]);

  useEffect(() => {
    if (dirty.current && !handedOff.current) autosave.schedule(draft);
  }, [autosave, draft]);

  // A window closed from outside (or the shell unmounting) still keeps the latest edits.
  useEffect(
    () => () => {
      if (!handedOff.current) void autosave.flush();
    },
    [autosave],
  );

  const handOff = (on: boolean) => {
    handedOff.current = on;
    if (on) void autosave.cancel();
  };

  const update = (fn: (d: DraftState) => DraftState) => {
    dirty.current = true;
    setDraft(fn);
  };

  const setIdentity = (identity: Identity) => {
    if (identity.accountId === draft.accountId) {
      update((d) => changeIdentity(d, identity));
      return;
    }
    // The old account's copy would linger in its Drafts otherwise.
    void autosave.cancel().then(() => saver.destroyCopy());
    const next = changeIdentity(draft, identity);
    update(() => next);
    upload(
      next.accountId,
      next.files.filter((f) => f.upload === 'pending'),
    );
  };

  const setRecipients = (kind: RecipientKind, list: ContactRef[]) =>
    update((d) => ({ ...d, [kind]: list }));

  const addFiles = (files: File[]) => {
    if (files.length === 0) return;
    const accountId = draft.accountId;
    const added: DraftAttachment[] = files.map((f) => ({
      local: f,
      upload: 'pending',
      file: {
        id: generateId(),
        aid: accountId,
        v: 0,
        __cls: 'File',
        messageId: draft.id,
        contentId: null,
        contentType: f.type || 'application/octet-stream',
        filename: f.name,
        size: f.size,
      },
    }));
    update((d) => ({ ...d, files: [...d.files, ...added] }));
    const total = [...draft.files, ...added].reduce((n, a) => n + a.file.size, 0);
    if (total > ATTACHMENT_WARN_BYTES) {
      toast.warning('Attachments are over 25 MB; many servers will refuse the message.');
    }
    upload(accountId, added);
  };

  /** Puts each placeholder's bytes to `accountId` and swaps in the engine's File. */
  const upload = (accountId: string, placeholders: DraftAttachment[]) => {
    const replace = (placeholder: string, next: DraftAttachment) =>
      update((d) => ({
        ...d,
        files: d.files.map((a) => (a.file.id === placeholder ? next : a)),
      }));
    for (const a of placeholders) {
      const local = a.local!;
      client.uploadFile(accountId, local, local.name).then(
        (file) => replace(a.file.id, { local, file }),
        (err: unknown) => {
          replace(a.file.id, { ...a, upload: 'failed' });
          toast.error(`Couldn't attach ${local.name}: ${errorText(err)}`);
        },
      );
    }
  };

  const removeFile = (id: string) =>
    update((d) => ({ ...d, files: d.files.filter((f) => f.file.id !== id) }));

  /**
   * SendDraftTask reads attachments from the engine's files directory by id, so a forwarded
   * original's files must be downloaded first; uploads are there already.
   */
  const prepareFiles = async (d: DraftState) => {
    for (const { file } of d.files) {
      try {
        await client.prepareFile(file);
      } catch (err) {
        throw new Error(`the attachment ${file.filename} isn't available (${errorText(err)})`, {
          cause: err,
        });
      }
    }
  };

  /** Merges recipients still being typed; null while attachments or addresses aren't ready. */
  const prepare = (pending: PendingRecipients): DraftState | null => {
    const current = latest.current;
    let next = current;
    const invalid: string[] = [];
    for (const kind of ['to', 'cc', 'bcc'] as const) {
      const r = pending.parsed[kind];
      if (!r) continue;
      invalid.push(...r.invalid);
      if (r.contacts.length)
        next = { ...next, [kind]: dedupeContacts([...next[kind], ...r.contacts]) };
    }
    if (next !== current) update(() => next);
    if (next.files.some((f) => f.upload === 'pending')) {
      toast('Wait for the attachments to finish uploading.');
      return null;
    }
    if (next.files.some((f) => f.upload === 'failed')) {
      toast.error('Remove the attachments that failed to upload.');
      return null;
    }
    if (invalid.length) {
      toast.error(`Check the address “${invalid[0]}”.`);
      return null;
    }
    return next;
  };

  /**
   * Saves the draft and asks the engine whether it can go out (`draft.validate`). Errors stop
   * it; warnings ask first, and `again` runs once the user says go ahead.
   */
  const check = async (
    d: DraftState,
    pending: PendingRecipients,
    opts: { ignoreWarnings?: boolean; again: () => void; label?: string },
  ): Promise<boolean> => {
    await autosave.cancel();
    await saver.save(d);
    const { errors, warnings } = await client.call('draft.validate', {
      accountId: d.accountId,
      headerMessageId: d.headerMessageId,
    });
    if (errors.length) {
      toast.error(errors.map((e) => e.message).join(' '));
      if (errors.some((e) => e.code === 'no-recipients')) pending.onNoRecipients?.();
      return false;
    }
    if (warnings.length && !opts.ignoreWarnings) {
      toast(warnings.map((w) => w.message).join(' '), {
        action: { label: opts.label ?? 'Send anyway', onClick: opts.again },
      });
      return false;
    }
    return true;
  };

  const send = (pending: PendingRecipients, opts: { ignoreWarnings?: boolean } = {}) => {
    if (busy) return;
    const d = prepare(pending);
    if (!d) return;
    setBusy(true);
    handOff(true);
    const again = () => send(NO_PENDING, { ignoreWarnings: true });
    check(d, pending, { ...opts, again }).then(
      (ok) => {
        if (!ok) return release();
        setComposerHidden(windowKey, true);
        sendByEngine(d, getUndoSendMs());
      },
      (err: unknown) => {
        toast.error(`Couldn't send: ${errorText(err)}`);
        release();
      },
    );
  };

  const release = () => {
    handOff(false);
    setBusy(false);
  };

  const reopen = () => {
    release();
    focusComposer(windowKey);
  };

  /**
   * `draft.send`, with undo send as the desktop does it: given a delay, the engine saves
   * `send-later` `{expiration, isUndoSend}` on the draft by its own clock and its executor
   * sends it when that comes due, so the send survives this tab closing. Undo is
   * `draft.cancelSend`.
   */
  const sendByEngine = (d: DraftState, delayMs: number) => {
    const watch = watchEngineSend(client, d, delayMs + ENGINE_SEND_WAIT_MS);
    const sent = prepareFiles(d).then(() =>
      client.call('draft.send', {
        accountId: d.accountId,
        headerMessageId: d.headerMessageId,
        ...(delayMs > 0 ? { delayMs } : {}),
      }),
    );
    let undone = false;
    let timer: ReturnType<typeof setTimeout> | undefined = undefined;
    const fail = (reason: string, toastId: string | number) => {
      watch.stop();
      clearTimeout(timer);
      toast.error(`Couldn't send: ${reason}`, { id: toastId, action: undefined, duration: 8000 });
      reopen();
    };
    const id =
      delayMs > 0
        ? toast('Sending…', {
            duration: delayMs,
            action: {
              label: 'Undo',
              onClick: () => {
                undone = true;
                watch.stop();
                clearTimeout(timer);
                toast.dismiss(id);
                // Later autosaves must not bring back a send-later row the draft had before.
                setDraft((cur) =>
                  cur.metadata.some((m) => m.pluginId === SEND_LATER_PLUGIN_ID)
                    ? setMetadata(cur, SEND_LATER_PLUGIN_ID, { expiration: null, isUndoSend: true })
                    : cur,
                );
                void sent
                  .then(() =>
                    client.call('draft.cancelSend', {
                      accountId: d.accountId,
                      headerMessageId: d.headerMessageId,
                    }),
                  )
                  .catch(() => undefined);
                reopen();
              },
            },
          })
        : toast.loading('Sending…', { duration: Infinity });
    sent.catch((err: unknown) => {
      if (!undone) fail(errorText(err), id);
    });
    timer = setTimeout(
      () => toast.loading('Sending…', { id, action: undefined, duration: Infinity }),
      delayMs,
    );
    void watch.done.then((o) => {
      if (undone) return;
      clearTimeout(timer);
      if (o.kind === 'failed') return fail(o.reason, id);
      if (o.kind === 'timeout') {
        toast(
          'Still sending. The engine keeps trying; the message stays in Drafts until it goes.',
          {
            id,
            action: undefined,
            duration: 8000,
          },
        );
        closeComposer(windowKey);
        return;
      }
      if (o.task.error)
        return fail(o.task.error.debuginfo ?? o.task.error.key ?? 'Unknown error', id);
      if (o.task.status === 'cancelled') return fail('The send was cancelled.', id);
      toast.success('Sent', { id, duration: 3000 });
      closeComposer(windowKey);
    });
  };

  const scheduleSend = async (
    at: Date,
    pending: PendingRecipients,
    opts: { ignoreWarnings?: boolean } = {},
  ) => {
    if (busy) return;
    const prepared = prepare(pending);
    if (!prepared) return;
    setBusy(true);
    handOff(true);
    const again = () => void scheduleSend(at, NO_PENDING, { ignoreWarnings: true });
    try {
      if (!(await check(prepared, pending, { ...opts, again, label: 'Schedule anyway' })))
        return release();
      const d = setMetadata(prepared, SEND_LATER_PLUGIN_ID, {
        expiration: Math.floor(at.getTime() / 1000),
      });
      await prepareFiles(d);
      // The engine's expiration executor sends it at `at`.
      await queueTask(client, tasks.syncbackDraft(draftToMessage(d)));
      closeComposer(windowKey);
      toast.success(`Scheduled for ${formatSendLater(at)}`);
    } catch (err) {
      release();
      toast.error(`Couldn't schedule the message: ${errorText(err)}`);
    }
  };

  const discard = async () => {
    await autosave.cancel();
    const hadCopy = saver.destroyCopy();
    closeComposer(windowKey);
    if (hadCopy) toast('Draft discarded');
  };

  /** Closes the window and keeps the draft (saved if it was edited). */
  const close = async () => {
    if (isPristine(draft)) {
      // An emptied draft is not worth keeping, here or in the engine.
      await autosave.cancel();
      saver.destroyCopy();
    } else if (dirty.current) {
      await autosave.flush();
    }
    closeComposer(windowKey);
  };

  /** Saves pending edits now. Returns whether the engine has a copy of the draft. */
  const keep = async (): Promise<boolean> => {
    if (dirty.current) await autosave.flush();
    return saver.hasCopy;
  };

  return {
    draft,
    update,
    keep,
    status,
    busy,
    setIdentity,
    setRecipients,
    addFiles,
    removeFile,
    send,
    scheduleSend,
    discard,
    close,
  };
}

/** Nothing worth keeping: no recipients, subject, text (beyond the signature) or files. */
function isPristine(d: DraftState): boolean {
  return (
    allRecipients(d).length === 0 &&
    !d.subject.trim() &&
    isBlankHtml(withoutSignature(d.body)) &&
    d.files.length === 0
  );
}

export type ComposerSession = ReturnType<typeof useComposerSession>;
