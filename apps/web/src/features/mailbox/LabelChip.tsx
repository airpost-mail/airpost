import { X } from 'lucide-react';
import type { CSSProperties } from 'react';
import { categoryDisplayName, type CategoryJSON } from '@/engine';
import { cn } from '@/lib/utils';
import { labelHue } from './labels';

const hueStyle = (cat: CategoryJSON) => ({ '--label-hue': labelHue(cat) }) as CSSProperties;

/** A 6px dot in the label's colour (sidebar). */
export function LabelDot({ label, className }: { label: CategoryJSON; className?: string }) {
  return (
    <span
      aria-hidden
      style={hueStyle(label)}
      className={cn(
        'size-1.5 shrink-0 rounded-full bg-[oklch(0.62_0.13_var(--label-hue))] dark:bg-[oklch(0.72_0.12_var(--label-hue))]',
        className,
      )}
    />
  );
}

/** A label pill: tinted fill, darker ink of the same hue. */
export function LabelChip({
  label,
  onRemove,
  className,
}: {
  label: CategoryJSON;
  onRemove?: () => void;
  className?: string;
}) {
  const name = categoryDisplayName(label);
  return (
    <span
      style={hueStyle(label)}
      className={cn(
        'inline-flex h-4.5 max-w-40 shrink-0 items-center gap-1 rounded-sm px-1.5 text-2xs font-medium',
        'bg-[oklch(0.94_0.035_var(--label-hue))] text-[oklch(0.42_0.1_var(--label-hue))]',
        'dark:bg-[oklch(0.3_0.05_var(--label-hue))] dark:text-[oklch(0.86_0.07_var(--label-hue))]',
        className,
      )}
    >
      <span className="truncate">{name}</span>
      {onRemove && (
        <button
          type="button"
          aria-label={`Remove label ${name}`}
          onClick={onRemove}
          className="-mr-0.5 rounded-xs opacity-60 hover:opacity-100 focus-visible:opacity-100"
        >
          <X className="size-3" />
        </button>
      )}
    </span>
  );
}
