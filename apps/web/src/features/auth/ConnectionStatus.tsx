import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { useEngine } from '@/engine';
import { cn } from '@/lib/utils';
import { useConnectionStatus, type ConnectionStatusInfo } from './connection';
import { serverLabel } from './server-url';
import { useSession } from './session';

function label({ status, state, online }: ConnectionStatusInfo): string {
  if (!online) return 'Offline';
  if (status === 'connected') return 'Connected';
  if (state === 'reconnecting') return 'Reconnecting';
  if (status === 'connecting') return 'Connecting';
  return 'Disconnected';
}

function detail(info: ConnectionStatusInfo, kind: 'mock' | 'ws', host: string | null): string {
  if (!info.online) return 'This browser has no network. Tern reconnects when it returns.';
  if (kind === 'mock') return 'In-browser demo engine with sample mail.';
  const where = host ? ` at ${host}` : '';
  if (info.status === 'connected') return `Engine${where} is up to date.`;
  if (info.status === 'connecting') return `Reaching the engine${where}.`;
  return `Couldn't reach the engine${where}.`;
}

/** Dot plus a short label for the top bar; the tooltip says where and why. */
export function ConnectionStatus({
  showLabel = true,
  className,
}: {
  showLabel?: boolean;
  className?: string;
}) {
  const engine = useEngine();
  const session = useSession();
  const info = useConnectionStatus();
  const text = label(info);
  const host = session?.server.url ? serverLabel(session.server.url) : null;

  return (
    <TooltipProvider delayDuration={300}>
      <Tooltip>
        <TooltipTrigger asChild>
          <span
            role="status"
            aria-label={`Engine: ${text}`}
            data-status={info.status}
            tabIndex={0}
            className={cn(
              'inline-flex h-6 items-center gap-1.5 rounded-sm px-1.5 font-mono text-2xs text-muted-foreground outline-offset-2',
              className,
            )}
          >
            <span className="relative flex size-1.5" aria-hidden>
              {info.status === 'connecting' && (
                <span className="absolute inset-0 animate-ping rounded-full bg-warning opacity-60" />
              )}
              <span
                className={cn(
                  'relative size-1.5 rounded-full transition-colors duration-(--duration-base)',
                  info.status === 'connected' && 'bg-success',
                  info.status === 'connecting' && 'bg-warning',
                  info.status === 'offline' && 'bg-destructive',
                )}
              />
            </span>
            {showLabel && (
              <span className="lowercase">
                {engine.kind === 'mock' && info.status === 'connected' ? 'demo' : text}
              </span>
            )}
          </span>
        </TooltipTrigger>
        <TooltipContent side="bottom" className="flex-col items-start gap-0.5">
          <span className="font-medium">{text}</span>
          <span className="text-background/70">{detail(info, engine.kind, host)}</span>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
