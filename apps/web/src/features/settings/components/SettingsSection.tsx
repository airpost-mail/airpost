import type { ReactNode } from 'react';
import { Label } from '@/components/ui/label';
import { cn } from '@/lib/utils';

/** A titled group of settings rows, drawn as one card with hairline separators. */
export function SettingsSection({
  title,
  description,
  action,
  children,
  className,
}: {
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn('flex flex-col gap-2', className)} aria-label={title}>
      <div className="flex items-end justify-between gap-4 px-1">
        <div>
          <h2 className="text-2xs font-medium tracking-widest text-muted-foreground uppercase">
            {title}
          </h2>
          {description && <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>}
        </div>
        {action}
      </div>
      <div className="divide-y divide-hairline rounded-lg border bg-card text-card-foreground">
        {children}
      </div>
    </section>
  );
}

/** Label and help on the left, the control on the right. */
export function SettingRow({
  label,
  description,
  htmlFor,
  children,
  className,
}: {
  label: ReactNode;
  description?: ReactNode;
  htmlFor?: string;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn('flex items-center justify-between gap-6 px-4 py-3', className)}>
      <div className="flex min-w-0 flex-col gap-0.5">
        {htmlFor ? (
          <Label htmlFor={htmlFor} className="text-sm font-medium">
            {label}
          </Label>
        ) : (
          <span className="text-sm font-medium">{label}</span>
        )}
        {description && <p className="text-xs text-muted-foreground">{description}</p>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}
