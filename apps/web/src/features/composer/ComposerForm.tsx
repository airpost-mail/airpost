import { Paperclip, Trash2 } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { flushSync } from 'react-dom';
import { ShortcutKeys } from '@/components/ShortcutKeys';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { useAccounts, type MessageJSON } from '@/engine';
import { useCapability } from '@/features/settings';
import { cn } from '@/lib/utils';
import { AttachmentList } from './AttachmentList';
import type { AutosaveStatus } from './autosave';
import { BodyEditor, type BodyEditorHandle } from './BodyEditor';
import { draftTitle, type DraftState } from './draft';
import { EditorToolbar } from './EditorToolbar';
import { FromSelect } from './FromSelect';
import { allIdentities } from './identities';
import { QuotedText } from './QuotedText';
import { RecipientField } from './RecipientField';
import { SendControls } from './SendControls';
import { getComposerWindows, setComposerView, type ComposerWindow } from './store';
import { TemplatesMenu } from './TemplatesMenu';
import {
  useComposerSession,
  type PendingRecipients,
  type RecipientFieldHandle,
} from './useComposerSession';
import { registerWindowActions } from './window-actions';
import { WindowHeader } from './WindowHeader';

const STATUS_TEXT: Partial<Record<AutosaveStatus, string>> = {
  pending: 'Draft',
  saving: 'Saving…',
  saved: 'Saved',
  error: "Couldn't save",
};

export function ComposerForm({
  win,
  initial,
  source,
}: {
  win: ComposerWindow;
  initial: { draft: DraftState; existing: boolean };
  source?: MessageJSON;
}) {
  const session = useComposerSession({ windowKey: win.key, initial, source });
  const { draft, update } = session;
  const toField = useRef<RecipientFieldHandle>(null);
  const ccField = useRef<RecipientFieldHandle>(null);
  const bccField = useRef<RecipientFieldHandle>(null);
  const { data: accounts = [] } = useAccounts();
  const editor = useRef<BodyEditorHandle>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const subjectInput = useRef<HTMLInputElement>(null);
  const [ccOpen, setCcOpen] = useState(draft.cc.length > 0);
  const [bccOpen, setBccOpen] = useState(draft.bcc.length > 0);
  const [linkOpen, setLinkOpen] = useState(false);
  const [dragging, setDragging] = useState(false);
  const minimised = win.view === 'minimised';
  const templates = useCapability('templates').available;

  const reveal = (kind: 'cc' | 'bcc') => {
    flushSync(() => (kind === 'cc' ? setCcOpen(true) : setBccOpen(true)));
    (kind === 'cc' ? ccField : bccField).current?.focus();
  };

  /** Recipients still being typed; committed to chips as part of sending. */
  const pending = (): PendingRecipients => ({
    parsed: {
      to: toField.current?.commit(),
      cc: ccField.current?.commit(),
      bcc: bccField.current?.commit(),
    },
    onNoRecipients: () => toField.current?.focus(),
  });

  const send = () => session.send(pending());

  const latest = useRef({ session, reveal, send });
  useEffect(() => {
    latest.current = { session, reveal, send };
  });

  useEffect(
    () =>
      registerWindowActions(win.key, {
        send: () => latest.current.send(),
        escape: () => {
          const view = getComposerWindows().find((w) => w.key === win.key)?.view;
          setComposerView(win.key, view === 'fullscreen' ? 'normal' : 'minimised');
        },
        showCc: () => latest.current.reveal('cc'),
        showBcc: () => latest.current.reveal('bcc'),
        discard: () => void latest.current.session.discard(),
      }),
    [win.key],
  );

  // Replies start in the body; new messages and forwards start with the recipients.
  const startInBody = win.mode === 'reply' || win.mode === 'reply-all' || win.mode === 'edit-draft';
  useEffect(() => {
    if (minimised) return;
    const id = requestAnimationFrame(() => {
      if (startInBody && draft.to.length) editor.current?.focus();
      else if (draft.to.length) subjectInput.current?.focus();
      else toField.current?.focus();
    });
    return () => cancelAnimationFrame(id);
    // Only when the window asks for focus, not on every edit.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [win.focusSeq]);

  const showFrom = allIdentities(accounts).length > 1;
  const used = [...draft.to, ...draft.cc, ...draft.bcc];

  const toggles = (
    <>
      {!ccOpen && (
        <Button
          size="xs"
          variant="ghost"
          className="text-muted-foreground"
          tabIndex={-1}
          onClick={() => reveal('cc')}
        >
          Cc
        </Button>
      )}
      {!bccOpen && (
        <Button
          size="xs"
          variant="ghost"
          className="text-muted-foreground"
          tabIndex={-1}
          onClick={() => reveal('bcc')}
        >
          Bcc
        </Button>
      )}
    </>
  );

  return (
    <>
      <WindowHeader
        windowKey={win.key}
        title={draftTitle(draft)}
        view={win.view}
        onClose={() => void session.close()}
        status={
          minimised &&
          STATUS_TEXT[session.status] && (
            <span className="font-mono text-2xs text-muted-foreground">
              {STATUS_TEXT[session.status]}
            </span>
          )
        }
      />
      <div
        className={cn('relative flex min-h-0 flex-1 flex-col', minimised && 'hidden')}
        onDragOver={(e) => {
          if (!e.dataTransfer.types.includes('Files')) return;
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
        }}
        onDrop={(e) => {
          if (!e.dataTransfer.files.length) return;
          e.preventDefault();
          setDragging(false);
          session.addFiles([...e.dataTransfer.files]);
        }}
      >
        {showFrom && (
          <FromSelect
            value={{ accountId: draft.accountId, contact: draft.from }}
            onChange={session.setIdentity}
          />
        )}
        <RecipientField
          ref={toField}
          label="To"
          value={draft.to}
          onChange={(list) => session.setRecipients('to', list)}
          exclude={used}
          trailing={toggles}
        />
        {ccOpen && (
          <RecipientField
            ref={ccField}
            label="Cc"
            value={draft.cc}
            onChange={(list) => session.setRecipients('cc', list)}
            exclude={used}
          />
        )}
        {bccOpen && (
          <RecipientField
            ref={bccField}
            label="Bcc"
            value={draft.bcc}
            onChange={(list) => session.setRecipients('bcc', list)}
            exclude={used}
          />
        )}
        <input
          ref={subjectInput}
          aria-label="Subject"
          placeholder="Subject"
          value={draft.subject}
          onChange={(e) => update((d) => ({ ...d, subject: e.target.value }))}
          className="h-10 shrink-0 border-b border-hairline bg-transparent px-(--pane-gutter) text-base font-medium outline-none placeholder:font-normal placeholder:text-muted-foreground"
        />
        <div
          className="min-h-0 flex-1 overflow-y-auto"
          onMouseDown={(e) => {
            // Clicking below the text still puts the caret in the body.
            if (e.target === e.currentTarget) {
              e.preventDefault();
              editor.current?.focus();
            }
          }}
        >
          <BodyEditor
            ref={editor}
            initialHtml={draft.body}
            onChange={(body) => update((d) => ({ ...d, body }))}
            onFiles={session.addFiles}
            onLinkRequest={() => setLinkOpen(true)}
          />
          <QuotedText
            html={draft.quoted}
            loading={draft.awaitingQuote}
            included={draft.includeQuoted}
            forwarded={!!draft.forwardedHeaderMessageId}
            onIncludedChange={(includeQuoted) => update((d) => ({ ...d, includeQuoted }))}
          />
          <AttachmentList files={draft.files} onRemove={session.removeFile} />
        </div>
        <footer className="flex shrink-0 items-center gap-1.5 border-t border-hairline px-(--pane-gutter) py-2">
          <SendControls
            busy={session.busy}
            onSend={send}
            onSchedule={(at) => void session.scheduleSend(at, pending())}
          />
          <div className="mx-1 h-4 w-px bg-border" aria-hidden />
          <EditorToolbar editor={editor} linkOpen={linkOpen} onLinkOpenChange={setLinkOpen} />
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                size="icon-xs"
                variant="ghost"
                aria-label="Attach files"
                onClick={() => fileInput.current?.click()}
                className="text-muted-foreground hover:text-foreground"
              >
                <Paperclip aria-hidden />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Attach files</TooltipContent>
          </Tooltip>
          {templates && (
            <TemplatesMenu
              current={{ subject: draft.subject, body: draft.body }}
              onInsert={(t) => {
                if (!draft.subject.trim() && t.subject)
                  update((d) => ({ ...d, subject: t.subject }));
                editor.current?.insertHtml(t.body);
              }}
            />
          )}
          <span
            className="ml-auto truncate font-mono text-2xs text-muted-foreground tabular-nums"
            aria-live="polite"
          >
            {STATUS_TEXT[session.status] ?? ''}
          </span>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                size="icon-xs"
                variant="ghost"
                aria-label="Discard draft"
                onClick={() => void session.discard()}
                className="text-muted-foreground hover:text-destructive"
              >
                <Trash2 aria-hidden />
              </Button>
            </TooltipTrigger>
            <TooltipContent className="flex items-center gap-2">
              Discard draft <ShortcutKeys keys="mod+shift+backspace" />
            </TooltipContent>
          </Tooltip>
          <input
            ref={fileInput}
            type="file"
            multiple
            hidden
            onChange={(e) => {
              session.addFiles([...(e.target.files ?? [])]);
              e.target.value = '';
            }}
          />
        </footer>
        {dragging && (
          <div className="pointer-events-none absolute inset-0 z-10 grid animate-in place-items-center rounded-b-sm border-2 border-dashed border-primary bg-background/85 duration-(--duration-fast) fade-in">
            <p className="text-2xl font-semibold tracking-tight">Drop to attach</p>
          </div>
        )}
      </div>
    </>
  );
}
