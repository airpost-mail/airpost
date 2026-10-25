// The reader's slot for inline replies: rendered under a thread's last message.
import { useRouter, type AnyRouter } from '@tanstack/react-router';
import { useEffect, useRef } from 'react';
import { cn } from '@/lib/utils';
import { ComposerForm } from './ComposerForm';
import { ComposerPlaceholder } from './ComposerWindow';
import { routeAccountId } from './route-account';
import {
  closeComposer,
  composerLabel,
  registerInlineSlot,
  setFocusedComposer,
  useComposerWindows,
  useFocusedComposer,
  type ComposerWindow,
} from './store';
import { useDraftSource } from './useDraftSource';

export function InlineComposer({ threadId }: { threadId: string }) {
  const windows = useComposerWindows();
  const router = useRouter({ warn: false }) as AnyRouter | undefined;
  useEffect(() => registerInlineSlot(threadId), [threadId]);

  const here = windows.filter((w) => w.placement === 'inline' && w.threadId === threadId);
  if (here.length === 0) return null;
  const fallbackAccountId = routeAccountId(router);
  return (
    <>
      {here.map((w) => (
        <InlineWindow key={w.key} win={w} fallbackAccountId={fallbackAccountId} />
      ))}
    </>
  );
}

function InlineWindow({
  win,
  fallbackAccountId,
}: {
  win: ComposerWindow;
  fallbackAccountId?: string;
}) {
  const { initial, error } = useDraftSource(win, fallbackAccountId);
  const focused = useFocusedComposer() === win.key;
  const el = useRef<HTMLElement>(null);

  useEffect(() => {
    if (win.focusSeq > 0 && !win.hidden)
      el.current?.scrollIntoView?.({ block: 'nearest', behavior: 'smooth' });
  }, [win.focusSeq, win.hidden, initial]);

  return (
    <section
      ref={el}
      aria-label={composerLabel(win.mode)}
      data-placement="inline"
      data-focused={focused || undefined}
      hidden={win.hidden}
      onFocusCapture={() => setFocusedComposer(win.key)}
      onBlurCapture={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocusedComposer(null);
      }}
      className={cn(
        'group/composer mx-6 mt-4 flex scroll-mb-6 flex-col rounded-lg border bg-popover text-popover-foreground',
        'transition-shadow duration-(--duration-fast) data-focused:border-ring/60',
      )}
    >
      {initial ? (
        <ComposerForm win={win} initial={initial} />
      ) : (
        <ComposerPlaceholder error={error} onClose={() => closeComposer(win.key)} />
      )}
    </section>
  );
}
