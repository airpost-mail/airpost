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
import { getPreferences } from '@/lib/preferences';
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
import { queueTask } from './engine-calls';
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
    if (identity.accountId !== draft.accountId) {
      // The old account's copy would linger in its Drafts otherwise.
      void autosave.cancel().then(() => saver.destroyCopy());
    }
    update((d) => changeIdentity(d, identity));
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
    const replace = (placeholder: string, next: DraftAttachment) =>
      update((d) => ({
        ...d,
        files: d.files.map((a) => (a.file.id === placeholder ? next : a)),
      }));
    for (const a of added) {
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
      handOff(false);
      setBusy(false);
      focusComposer(windowKey);
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
    const delayMs = getPreferences().undoSendMs;
    // TODO(engine): undo-send is a client-side delay. If the tab closes inside it the draft
    // stays saved but unsent. With DCP stage 4's executor the web writes send-later
    // `{expiration, isUndoSend}` metadata instead; stage 13's `draft.send {delayMs}` later.
    if (delayMs <= 0) {
      setComposerHidden(windowKey, true);
      void deliver(d, toast.loading('Sending…'));
      return;
    }
    setComposerHidden(windowKey, true);
    // Saved first so an undo finds the text even if this composer unmounted meanwhile (an
    // inline reply whose thread was left reopens as its saved draft).
    const saved = autosave
      .cancel()
      .then(() => saver.save(d))
      .catch(() => undefined);
    let timer: ReturnType<typeof setTimeout> | undefined = undefined;
    const id = toast('Sending…', {
      duration: delayMs,
      action: {
        label: 'Undo',
        onClick: () => {
          clearTimeout(timer);
          handOff(false);
          setBusy(false);
          focusComposer(windowKey);
          toast.dismiss(id);
        },
      },
    });
    timer = setTimeout(() => void saved.then(() => deliver(d, id)), delayMs);
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
      // The engine fires a metadata-expiration delta at `at`; useSendLaterRunner sends it.
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
