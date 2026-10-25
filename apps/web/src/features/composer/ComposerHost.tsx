// Mounted once by the shell: the dock of composer windows and the composer shortcuts.
import { useRouter, type AnyRouter } from '@tanstack/react-router';
import { useShortcut, useShortcutScope } from '@/lib/shortcuts';
import { ComposerWindow } from './ComposerWindow';
import { routeAccountId } from './route-account';
import {
  getFocusedComposer,
  getComposerWindows,
  openComposer,
  setComposerView,
  useComposerWindows,
  useFocusedComposer,
} from './store';
import { windowActions } from './window-actions';

/** The window composer shortcuts act on: the focused one, else a full-screen one. */
function targetKey(): string | null {
  return (
    getFocusedComposer() ??
    getComposerWindows().find((w) => w.view === 'fullscreen' && !w.hidden)?.key ??
    null
  );
}

const inComposer = { scope: 'composer', allowInInputs: true, group: 'Compose' } as const;

export function ComposerHost() {
  const windows = useComposerWindows().filter((w) => w.placement === 'docked');
  const focused = useFocusedComposer();
  const router = useRouter({ warn: false }) as AnyRouter | undefined;
  const fullscreen = windows.find((w) => w.view === 'fullscreen' && !w.hidden);

  useShortcutScope('composer', !!focused || !!fullscreen);

  useShortcut('c', () => openComposer({ mode: 'new', accountId: routeAccountId(router) }), {
    description: 'Compose',
    group: 'Compose',
  });
  useShortcut('mod+enter', () => windowActions(targetKey())?.send(), {
    ...inComposer,
    description: 'Send',
  });
  useShortcut('escape', () => windowActions(targetKey())?.escape(), {
    ...inComposer,
    description: 'Minimise composer',
  });
  useShortcut('mod+shift+c', () => windowActions(targetKey())?.showCc(), {
    ...inComposer,
    description: 'Add Cc',
  });
  useShortcut('mod+shift+b', () => windowActions(targetKey())?.showBcc(), {
    ...inComposer,
    description: 'Add Bcc',
  });
  useShortcut('mod+shift+backspace', () => windowActions(targetKey())?.discard(), {
    ...inComposer,
    description: 'Discard draft',
  });

  if (windows.length === 0) return null;
  const fallbackAccountId = routeAccountId(router);

  return (
    <>
      {fullscreen && (
        <div
          aria-hidden
          className="fixed inset-0 z-40 animate-in bg-foreground/25 duration-(--duration-slow) fade-in"
          onClick={() => setComposerView(fullscreen.key, 'normal')}
        />
      )}
      <div className="pointer-events-none fixed right-0 bottom-0 z-40 flex max-w-full items-end gap-3 px-4">
        {windows.map((w) => (
          <ComposerWindow key={w.key} win={w} fallbackAccountId={fallbackAccountId} />
        ))}
      </div>
    </>
  );
}
