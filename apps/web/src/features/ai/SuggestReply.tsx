import { Sparkles, Square } from 'lucide-react';
import { useEffect, useRef, useState, type RefObject } from 'react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { useEngine, type AiSuggestion } from '@/engine';
import type { BodyEditorHandle } from '@/features/composer/BodyEditor';
import { errorMessage, isCancelled, startAiCall, type AiCall } from './api';
import { destinationNote, providerName } from './config';
import { unavailableMessage, useAiAvailability } from './hooks';
import { insertReply } from './reply-insert';

type Phase =
  | { kind: 'idle' }
  | { kind: 'suggesting' }
  | { kind: 'writing'; text: string }
  | { kind: 'error'; message: string };

/**
 * Suggest in a reply composer: up to three one-line suggestions and a field for what to say.
 * Picking one or writing an instruction streams a full reply, which goes into the body when
 * it is complete. Nothing is sent to the model until the user asks, and nothing is sent as
 * mail.
 */
export function SuggestReply({
  accountId,
  threadId,
  editor,
}: {
  accountId: string;
  threadId: string;
  editor: RefObject<BodyEditorHandle | null>;
}) {
  const client = useEngine();
  const ai = useAiAvailability('reply', accountId);
  const [open, setOpen] = useState(false);
  const [phase, setPhase] = useState<Phase>({ kind: 'idle' });
  const [suggestions, setSuggestions] = useState<AiSuggestion[] | null>(null);
  const [instruction, setInstruction] = useState('');
  const call = useRef<AiCall<unknown> | null>(null);

  const stop = () => {
    call.current?.cancel();
    call.current = null;
  };
  useEffect(() => () => call.current?.cancel(), []);

  if (!ai.available && (ai.reason === 'engine' || ai.reason === 'off' || ai.reason === 'account'))
    return null;
  const name = ai.provider ? providerName(ai.provider) : undefined;
  const where = destinationNote(ai.provider);
  const busy = phase.kind === 'suggesting' || phase.kind === 'writing';

  const fail = (err: unknown) => {
    setPhase(
      isCancelled(err) ? { kind: 'idle' } : { kind: 'error', message: errorMessage(err, name) },
    );
  };

  const suggest = () => {
    stop();
    setPhase({ kind: 'suggesting' });
    const c = startAiCall(client, 'ai.suggestReplies', { accountId, threadId, count: 3 });
    call.current = c;
    c.promise.then(
      (list) => {
        if (call.current !== c) return;
        call.current = null;
        setSuggestions(list);
        setPhase({ kind: 'idle' });
      },
      (err: unknown) => call.current === c && fail(err),
    );
  };

  const write = (p: { suggestion?: string; instruction?: string }) => {
    stop();
    setPhase({ kind: 'writing', text: '' });
    let text = '';
    const c = startAiCall(client, 'ai.draftReply', { accountId, threadId, ...p }, (piece) => {
      text += piece;
      setPhase({ kind: 'writing', text });
    });
    call.current = c;
    c.promise.then(
      (r) => {
        if (call.current !== c) return;
        call.current = null;
        if (editor.current) insertReply(editor.current, r.text);
        setPhase({ kind: 'idle' });
        setSuggestions(null);
        setInstruction('');
        setOpen(false);
      },
      (err: unknown) => call.current === c && fail(err),
    );
  };

  const onOpenChange = (next: boolean) => {
    // Closing the popover stops whatever it asked for.
    if (!next) {
      stop();
      setPhase({ kind: 'idle' });
    }
    setOpen(next);
  };

  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              size="icon-xs"
              variant="ghost"
              aria-label="Suggest a reply"
              disabled={!ai.available}
              className="text-muted-foreground hover:text-foreground"
            >
              <Sparkles aria-hidden />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>
          {ai.available ? `Suggest a reply. ${where}` : unavailableMessage(ai.reason)}
        </TooltipContent>
      </Tooltip>
      <PopoverContent align="start" className="flex w-96 flex-col gap-3">
        <p className="text-2xs text-muted-foreground">{where}</p>
        {suggestions === null ? (
          <Button size="sm" variant="outline" disabled={busy} onClick={suggest}>
            {phase.kind === 'suggesting' ? 'Thinking…' : 'Get suggestions'}
          </Button>
        ) : suggestions.length === 0 ? (
          <p className="text-xs text-muted-foreground">No suggestions for this thread.</p>
        ) : (
          <ul className="flex flex-col gap-1" aria-label="Suggestions">
            {suggestions.map((s, i) => (
              <li key={i}>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => write({ suggestion: s.text })}
                  className="flex w-full flex-col items-start rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted disabled:opacity-50"
                >
                  <span className="font-medium">{s.label}</span>
                  <span className="text-xs text-muted-foreground">{s.text}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (instruction.trim()) write({ instruction: instruction.trim() });
          }}
        >
          <Input
            aria-label="What to say"
            placeholder="Say I'll call Tuesday"
            value={instruction}
            disabled={busy}
            onChange={(e) => setInstruction(e.target.value)}
            className="h-7 text-sm"
          />
          <Button type="submit" size="sm" disabled={busy || !instruction.trim()}>
            Write
          </Button>
        </form>
        {phase.kind === 'writing' && (
          <div className="flex flex-col gap-2 rounded-md border bg-muted/40 p-2" aria-busy>
            {/* Plain text only (docs/ai-plan.md A6). */}
            <p className="max-h-48 overflow-y-auto text-sm break-words whitespace-pre-wrap">
              {phase.text || 'Writing…'}
            </p>
            <Button
              size="xs"
              variant="ghost"
              className="self-end"
              onClick={() => {
                stop();
                setPhase({ kind: 'idle' });
              }}
            >
              <Square aria-hidden />
              Stop
            </Button>
          </div>
        )}
        {phase.kind === 'error' && (
          <p role="status" className="text-xs text-muted-foreground">
            {phase.message}
          </p>
        )}
      </PopoverContent>
    </Popover>
  );
}
