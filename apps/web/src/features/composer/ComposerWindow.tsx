import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { cn } from '@/lib/utils';
import { ComposerForm } from './ComposerForm';
import {
  closeComposer,
  setFocusedComposer,
  useFocusedComposer,
  type ComposerView,
  type ComposerWindow as ComposerWindowState,
} from './store';
import { useDraftSource } from './useDraftSource';
import { WindowHeader } from './WindowHeader';

const VIEW_CLASSES: Record<ComposerView, string> = {
  normal: 'h-[min(33rem,calc(100dvh-4rem))] w-[34rem]',
  maximised: 'h-[calc(100dvh-3.5rem)] w-[46rem]',
  minimised: 'w-72',
  fullscreen:
    'fixed inset-3 z-50 rounded-lg border-b sm:inset-x-[max(1.5rem,calc((100vw-60rem)/2))] sm:inset-y-8',
};

export function ComposerWindow({
  win,
  fallbackAccountId,
}: {
  win: ComposerWindowState;
  fallbackAccountId?: string;
}) {
  const { initial, source, error } = useDraftSource(win, fallbackAccountId);
  const focused = useFocusedComposer() === win.key;
  const label =
    win.mode === 'new'
      ? 'New message'
      : win.mode === 'forward'
        ? 'Forward'
        : win.mode === 'edit-draft'
          ? 'Draft'
          : 'Reply';

  return (
    <section
      role="dialog"
      aria-modal={win.view === 'fullscreen' || undefined}
      aria-label={label}
      data-focused={focused || undefined}
      hidden={win.hidden}
      onFocusCapture={() => setFocusedComposer(win.key)}
      onBlurCapture={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocusedComposer(null);
      }}
      className={cn(
        'group/composer pointer-events-auto relative flex max-w-[calc(100vw-2rem)] animate-in flex-col rounded-t-lg border border-b-0 bg-popover text-popover-foreground shadow-xl shadow-foreground/10 duration-(--duration-base) ease-(--ease-out) fade-in slide-in-from-bottom-2',
        VIEW_CLASSES[win.view],
      )}
    >
      {initial ? (
        <ComposerForm win={win} initial={initial} source={source} />
      ) : (
        <>
          <WindowHeader
            windowKey={win.key}
            title={label}
            view={win.view}
            onClose={() => closeComposer(win.key)}
          />
          {win.view !== 'minimised' &&
            (error ? (
              <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
                <p className="text-2xl font-semibold tracking-tight text-foreground/80">
                  Not this time
                </p>
                <p className="text-sm text-muted-foreground">{error}</p>
                <Button size="sm" variant="secondary" onClick={() => closeComposer(win.key)}>
                  Close
                </Button>
              </div>
            ) : (
              <div aria-busy className="flex flex-col gap-3 p-(--pane-gutter)">
                <Skeleton className="h-4 w-2/3" />
                <Skeleton className="h-4 w-1/2" />
                <Skeleton className="h-24 w-full" />
              </div>
            ))}
        </>
      )}
    </section>
  );
}
