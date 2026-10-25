// The state and actions of one composer window: edits, autosave, send with undo, schedule,
// discard. UI components only call into this.
import { useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import {
  generateId,
  tasks,
  useEngine,
  waitForTask,
  type ContactRef,
  type MessageJSON,
} from '@/engine';
import { getUndoSendMs } from '@/lib/undo-send';
import type { AutosaveStatus } from './autosave';
import {
  allRecipients,
  changeIdentity,
  draftToMessage,
  setMetadata,
  withQuoteFrom,
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
  source?: MessageJSON;
}) {
  const { windowKey, initial, source } = args;
  const client = useEngine();
  const [draft, setDraft] = useState(initial.draft);
  const [status, setStatus] = useState<AutosaveStatus>(initial.existing ? 'saved' : 'idle');
  const [busy, setBusy] = useState(false);
  const dirty = useRef(false);
  // Set while a send or schedule owns the draft: a save queued after SendDraftTask would
  // bring the sent message back as a draft.
  const handedOff = useRef(false);

  // The quoted original fills in when its body arrives; that is not a user edit.
  if (draft.awaitingQuote && source?.body != null) setDraft(withQuoteFrom(draft, source));

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

  /** Merges recipients still being typed and checks the draft can go out. */
  const prepare = (pending: PendingRecipients): DraftState | null => {
    let next = draft;
    const invalid: string[] = [];
    for (const kind of ['to', 'cc', 'bcc'] as const) {
      const r = pending.parsed[kind];
      if (!r) continue;
      invalid.push(...r.invalid);
      if (r.contacts.length)
        next = { ...next, [kind]: dedupeContacts([...next[kind], ...r.contacts]) };
    }
    if (next !== draft) update(() => next);
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
    if (allRecipients(next).length === 0) {
      toast.error('Add at least one recipient.');
      pending.onNoRecipients?.();
      return null;
    }
    return next;
  };

  const deliver = async (d: DraftState, toastId: string | number) => {
    toast.loading('Sending…', { id: toastId, action: undefined, duration: Infinity });
    try {
      await prepareFiles(d);
      const task = tasks.sendDraft(draftToMessage(d));
      const done = waitForTask(client, task.id);
      await queueTask(client, task);
      const result = await done;
      if (result.error)
        throw new Error(result.error.debuginfo ?? result.error.key ?? 'Unknown error');
      if (result.status === 'cancelled') throw new Error('The send was cancelled.');
      toast.success('Sent', { id: toastId, duration: 3000 });
      closeComposer(windowKey);
    } catch (err) {
      toast.error(`Couldn't send: ${errorText(err)}`, {
        id: toastId,
        duration: 8000,
      });
      reopen();
    }
  };

  const send = (pending: PendingRecipients, opts: { allowEmptySubject?: boolean } = {}) => {
    if (busy) return;
    const d = prepare(pending);
    if (!d) return;
    if (!opts.allowEmptySubject && !d.subject.trim()) {
      toast('Send without a subject?', {
        action: { label: 'Send', onClick: () => send(NO_PENDING, { allowEmptySubject: true }) },
      });
      return;
    }
    setBusy(true);
    handOff(true);
    const delayMs = getUndoSendMs();
    if (delayMs <= 0) {
      setComposerHidden(windowKey, true);
      void deliver(d, toast.loading('Sending…'));
      return;
    }
    setComposerHidden(windowKey, true);
    sendByEngine(d, delayMs);
  };

  const reopen = () => {
    handOff(false);
    setBusy(false);
    focusComposer(windowKey);
  };

  /**
   * Undo send as the desktop does it (DCP 3.3): the draft is saved with `send-later`
   * `{expiration, isUndoSend}` and the engine's executor sends it when that comes due, so the
   * send survives this tab closing. Undo clears the expiration first. The expiration is this
   * browser's clock, so a clock far off the engine's shortens or stretches the wait.
   */
  const sendByEngine = (d: DraftState, delayMs: number) => {
    const queued = setMetadata(d, SEND_LATER_PLUGIN_ID, {
      expiration: Math.ceil((Date.now() + delayMs) / 1000),
      isUndoSend: true,
    });
    const watch = watchEngineSend(client, d, delayMs + ENGINE_SEND_WAIT_MS);
    const saved = autosave.cancel().then(async () => {
      await prepareFiles(queued);
      await saver.save(queued);
    });
    let undone = false;
    let timer: ReturnType<typeof setTimeout> | undefined = undefined;
    const fail = (reason: string, toastId: string | number) => {
      watch.stop();
      clearTimeout(timer);
      toast.error(`Couldn't send: ${reason}`, { id: toastId, action: undefined, duration: 8000 });
      reopen();
    };
    const id = toast('Sending…', {
      duration: delayMs,
      action: {
        label: 'Undo',
        onClick: () => {
          undone = true;
          watch.stop();
          clearTimeout(timer);
          toast.dismiss(id);
          const cleared = setMetadata(queued, SEND_LATER_PLUGIN_ID, {
            expiration: null,
            isUndoSend: true,
          });
          // Later autosaves carry the cleared row rather than the one that sends.
          setDraft((cur) => ({ ...cur, metadata: cleared.metadata }));
          void saved
            .then(() =>
              queueTask(
                client,
                tasks.syncbackMetadata({
                  accountId: d.accountId,
                  pluginId: SEND_LATER_PLUGIN_ID,
                  modelId: d.id,
                  modelClassName: 'Message',
                  modelHeaderMessageId: d.headerMessageId,
                  value: { expiration: null, isUndoSend: true },
                }),
              ),
            )
            .catch(() => undefined);
          reopen();
        },
      },
    });
    saved.catch((err: unknown) => {
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

  const scheduleSend = async (at: Date, pending: PendingRecipients) => {
    const prepared = prepare(pending);
    if (!prepared) return;
    const d = setMetadata(prepared, SEND_LATER_PLUGIN_ID, {
      expiration: Math.floor(at.getTime() / 1000),
    });
    setBusy(true);
    handedOff.current = true;
    await autosave.cancel();
    try {
      await prepareFiles(d);
      // The engine's expiration executor sends it at `at`.
      await queueTask(client, tasks.syncbackDraft(draftToMessage(d)));
      closeComposer(windowKey);
      toast.success(`Scheduled for ${formatSendLater(at)}`);
    } catch (err) {
      handedOff.current = false;
      setBusy(false);
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
