import { cn } from '@/lib/utils';

export type Tone = 'success' | 'warning' | 'destructive' | 'muted' | 'primary';

const TONE: Record<Tone, string> = {
  success: 'bg-success',
  warning: 'bg-warning',
  destructive: 'bg-destructive',
  muted: 'bg-muted-foreground/40',
  primary: 'bg-primary',
};

/** A small dot and a short word: "Ready", "Needs a key", "Offline". */
export function Status({ tone, children }: { tone: Tone; children: string }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
      <span aria-hidden className={cn('size-1.5 rounded-full', TONE[tone])} />
      {children}
    </span>
  );
}
