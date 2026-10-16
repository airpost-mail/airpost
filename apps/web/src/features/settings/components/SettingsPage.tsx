import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

/** One settings screen: serif title, a line of help, then sections. */
export function SettingsPage({
  title,
  description,
  actions,
  children,
  className,
}: {
  title: string;
  description?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn('flex animate-in flex-col gap-8 duration-200 fade-in', className)}>
      <header className="flex items-end justify-between gap-4 border-b border-hairline pb-4">
        <div className="min-w-0">
          <h1 className="text-display font-semibold tracking-tight">{title}</h1>
          {description && <p className="mt-1 text-sm text-muted-foreground">{description}</p>}
        </div>
        {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      </header>
      {children}
    </div>
  );
}
