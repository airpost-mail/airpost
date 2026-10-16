import { CloudOff, LoaderCircle, WifiOff } from 'lucide-react';
import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { ensureConnected, useEngine } from '@/engine';
import { cn } from '@/lib/utils';
import { useAfterDelay, useConnectionStatus } from './connection';

/** Below this, a connect is quick enough that a banner would only flicker. */
const CONNECTING_GRACE_MS = 1500;

/**
 * A strip for the top of the shell when the engine is unreachable or reconnecting. Renders
 * nothing while connected.
 */
export function OfflineBanner({ className }: { className?: string }) {
  const engine = useEngine();
  const { state, online } = useConnectionStatus();
  const slowConnect = useAfterDelay(state === 'connecting', CONNECTING_GRACE_MS);
  const [retrying, setRetrying] = useState(false);

  let tone: 'offline' | 'pending';
  let message: string;
  if (!online) {
    tone = 'offline';
    message = "You're offline. Tern reconnects when the network returns.";
  } else if (state === 'closed') {
    tone = 'offline';
    message = "Couldn't reach the engine.";
  } else if (state === 'reconnecting') {
    tone = 'pending';
    message = 'Reconnecting to the engine…';
  } else if (slowConnect) {
    tone = 'pending';
    message = 'Connecting to the engine…';
  } else {
    return null;
  }

  const retry = () => {
    setRetrying(true);
    ensureConnected(engine)
      .catch(() => undefined)
      .finally(() => setRetrying(false));
  };

  const Icon = tone === 'pending' ? LoaderCircle : online ? CloudOff : WifiOff;
  return (
    <div
      role="status"
      aria-live="polite"
      data-tone={tone}
      className={cn(
        'flex h-8 shrink-0 animate-in items-center gap-2 border-b px-(--pane-gutter) text-xs duration-(--duration-base) fade-in slide-in-from-top-1',
        tone === 'offline'
          ? 'border-destructive/25 bg-destructive/8 text-destructive'
          : 'border-warning/30 bg-warning/10 text-foreground',
        className,
      )}
    >
      <Icon className={cn('size-3.5 shrink-0', tone === 'pending' && 'animate-spin')} aria-hidden />
      <span className="min-w-0 truncate">{message}</span>
      {online && state === 'closed' && (
        <Button
          variant="ghost"
          size="xs"
          className="ml-auto text-current hover:bg-destructive/10 hover:text-current"
          onClick={retry}
          disabled={retrying}
        >
          {retrying ? 'Retrying…' : 'Retry'}
        </Button>
      )}
    </div>
  );
}
