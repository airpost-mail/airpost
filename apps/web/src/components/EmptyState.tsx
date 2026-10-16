import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

/** Calm, editorial empty state: serif headline, one line of help, optional action. */
export function EmptyState({
  title,
  description,
  action,
  className,
}: {
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        'flex h-full animate-in flex-col items-center justify-center gap-2 px-6 text-center duration-300 fade-in',
        className,
      )}
    >
      <p className="text-display font-semibold tracking-tight text-foreground/80">{title}</p>
      {description && <p className="max-w-sm text-sm text-muted-foreground">{description}</p>}
      {action && <div className="mt-3">{action}</div>}
    </div>
  );
}
