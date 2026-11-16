import { cn } from '@/lib/utils';

/** The airpost mark, swapped for the dark-background variant in dark mode. */
export function LogoMark({ className }: { className?: string }) {
  return (
    <span className={cn('relative inline-block size-6 shrink-0', className)} aria-hidden>
      <img src="/brand/airpost-bird-on-light-64.png" alt="" className="size-full dark:hidden" />
      <img src="/brand/airpost-bird-on-dark-64.png" alt="" className="hidden size-full dark:block" />
    </span>
  );
}

export function Wordmark({ className }: { className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-1.5', className)}>
      <LogoMark />
      <span className="text-xl leading-none font-semibold tracking-tight">Airpost</span>
    </span>
  );
}
