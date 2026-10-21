import { RotateCw, Sparkles, Square, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Button } from '@/components/ui/button';
import { useEngine, type MessageJSON, type ThreadJSON } from '@/engine';
import { errorMessage, isCancelled, summarize, type AiCall } from './api';
import { destinationNote, providerName } from './config';
import { unavailableMessage, useAiAvailability } from './hooks';
import {
  remember,
  remembered,
  watchSummaries,
  worthSummarizing,
  type Summary,
} from './summary-memory';
import { TriageChip } from './TriageChip';

type State =
  | { kind: 'idle' }
  | { kind: 'streaming'; text: string }
  | { kind: 'done'; summary: Summary }
  | { kind: 'error'; message: string; text: string };

/** The reader's AI row: the triage chip, Summarize, and the summary card under them. */
export function ThreadAi({ thread, messages }: { thread: ThreadJSON; messages?: MessageJSON[] }) {
  const sent = (messages ?? []).filter((m) => !m.draft);
  const newest = sent[sent.length - 1]?.id ?? '';
  return (
    <>
      <div className="flex flex-wrap items-center gap-1.5 empty:hidden">
        <TriageChip thread={thread} />
      </div>
      {messages && worthSummarizing(messages) && (
        <SummaryCard
          key={`${thread.id}:${newest}`}
          thread={thread}
          memoryKey={`${thread.id}:${newest}`}
        />
      )}
    </>
  );
}

function SummaryCard({ thread, memoryKey }: { thread: ThreadJSON; memoryKey: string }) {
  const client = useEngine();
  const ai = useAiAvailability('summary', thread.aid);
  const [state, setState] = useState<State>(() => {
    const known = remembered.get(memoryKey);
    return known ? { kind: 'done', summary: known } : { kind: 'idle' };
  });
  const [open, setOpen] = useState(() => remembered.has(memoryKey));
  const call = useRef<AiCall<unknown> | null>(null);

  useEffect(() => watchSummaries(client), [client]);
  // Closing the thread or the card stops the provider request.
  useEffect(() => () => call.current?.cancel(), []);

  if (!ai.available && ai.reason === 'engine') return null;
  if (!ai.available && (ai.reason === 'off' || ai.reason === 'account')) return null;
  const name = ai.provider ? providerName(ai.provider) : undefined;
  const where = destinationNote(ai.provider);

  const run = (refresh: boolean) => {
    call.current?.cancel();
    setOpen(true);
    setState({ kind: 'streaming', text: '' });
    let text = '';
    const c = summarize(
      client,
      { accountId: thread.aid, threadId: thread.id, refresh },
      (piece) => {
        text += piece;
        setState({ kind: 'streaming', text });
      },
    );
    call.current = c;
    c.promise.then(
      (r) => {
        if (call.current !== c) return;
        call.current = null;
        const summary = { text: r.text, model: r.model, where };
        remember(memoryKey, summary);
        setState({ kind: 'done', summary });
      },
      (err: unknown) => {
        if (call.current !== c) return;
        call.current = null;
        setState({
          kind: 'error',
          message: errorMessage(err, name),
          text: isCancelled(err) ? text : '',
        });
      },
    );
  };
  const stop = () => {
    call.current?.cancel();
  };
  const close = () => {
    call.current?.cancel();
    call.current = null;
    setOpen(false);
    if (state.kind !== 'done') setState({ kind: 'idle' });
  };

  if (!open) {
    return (
      <div className="flex items-center gap-2">
        <Button
          size="xs"
          variant="outline"
          disabled={!ai.available}
          onClick={() => run(false)}
          title={ai.available ? where : unavailableMessage(ai.reason)}
        >
          <Sparkles aria-hidden />
          Summarize
        </Button>
        <span className="text-2xs text-muted-foreground">
          {ai.available ? where : unavailableMessage(ai.reason)}
        </span>
      </div>
    );
  }

  const text = state.kind === 'done' ? state.summary.text : state.kind === 'idle' ? '' : state.text;
  return (
    <section
      aria-label="Summary"
      aria-busy={state.kind === 'streaming'}
      className="flex flex-col gap-2 rounded-lg border bg-card px-4 py-3 text-card-foreground"
    >
      <header className="flex items-center gap-2">
        <Sparkles className="size-3.5 text-primary" aria-hidden />
        <h2 className="text-2xs font-medium tracking-widest text-muted-foreground uppercase">
          Summary
        </h2>
        <span className="min-w-0 flex-1 truncate text-2xs text-muted-foreground">
          {(state.kind === 'done' ? [state.summary.model, state.summary.where] : [where])
            .filter(Boolean)
            .join(' · ')}
        </span>
        {state.kind === 'streaming' ? (
          <Button size="xs" variant="ghost" onClick={stop}>
            <Square aria-hidden />
            Stop
          </Button>
        ) : (
          <Button size="xs" variant="ghost" disabled={!ai.available} onClick={() => run(true)}>
            <RotateCw aria-hidden />
            Regenerate
          </Button>
        )}
        <Button size="icon-xs" variant="ghost" aria-label="Close summary" onClick={close}>
          <X aria-hidden />
        </Button>
      </header>
      {/* Model output is plain text: no links, no HTML. */}
      {text && <p className="text-sm break-words whitespace-pre-wrap">{text}</p>}
      {state.kind === 'streaming' && !text && (
        <p className="text-sm text-muted-foreground">Reading the thread…</p>
      )}
      {state.kind === 'error' && (
        <p role="status" className="text-xs text-muted-foreground">
          {state.message}
        </p>
      )}
    </section>
  );
}
