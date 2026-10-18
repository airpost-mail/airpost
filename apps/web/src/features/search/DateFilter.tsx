import { addDays, format, subDays } from 'date-fns';
import { CalendarRange } from 'lucide-react';
import { useState } from 'react';
import type { DateRange as DayRange } from 'react-day-picker';
import { Button } from '@/components/ui/button';
import { Calendar } from '@/components/ui/calendar';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import { formatDateValue, parseDateValue } from './query';

export interface DateBounds {
  /** `after:` value, inclusive. */
  after?: string;
  /** `before:` value, exclusive. */
  before?: string;
}

const PRESETS: { label: string; bounds: DateBounds }[] = [
  { label: 'Any time', bounds: {} },
  { label: 'Today', bounds: { after: 'today' } },
  { label: 'Last 7 days', bounds: { after: '7d' } },
  { label: 'Last 30 days', bounds: { after: '30d' } },
  { label: 'Last year', bounds: { after: '1y' } },
];

function summary(bounds: DateBounds, now: Date): string | null {
  const preset = PRESETS.find(
    (p) => p.bounds.after === bounds.after && p.bounds.before === bounds.before,
  );
  if (preset && preset.label !== 'Any time') return preset.label;
  const a = bounds.after ? parseDateValue(bounds.after, now) : null;
  const b = bounds.before ? parseDateValue(bounds.before, now) : null;
  if (a && b) return `${format(a, 'd MMM')} – ${format(subDays(b, 1), 'd MMM yyyy')}`;
  if (a) return `Since ${format(a, 'd MMM yyyy')}`;
  if (b) return `Before ${format(b, 'd MMM yyyy')}`;
  return null;
}

/** One control for both bounds: presets plus a range calendar. */
export function DateFilter({
  bounds,
  disabled,
  onChange,
}: {
  bounds: DateBounds;
  /** The query's dates sit inside an OR, NOT or parentheses: edit them in the text. */
  disabled?: boolean;
  onChange: (bounds: DateBounds) => void;
}) {
  const [open, setOpen] = useState(false);
  const now = new Date();
  const after = bounds.after ? parseDateValue(bounds.after, now) : null;
  const before = bounds.before ? parseDateValue(bounds.before, now) : null;
  const selected: DayRange | undefined = after
    ? { from: after, to: before ? subDays(before, 1) : undefined }
    : undefined;
  const label = summary(bounds, now);

  const pick = (b: DateBounds) => {
    onChange(b);
    setOpen(false);
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          disabled={disabled}
          className={cn('h-6 gap-1.5 px-2 text-xs', label && 'border-primary/40 text-primary')}
        >
          <CalendarRange className="size-3.5" />
          {label ?? 'Date'}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-auto flex-row gap-0 p-0">
        <ul className="flex w-32 flex-col gap-0.5 border-r border-hairline p-1.5">
          {PRESETS.map((p) => (
            <li key={p.label}>
              <button
                type="button"
                onClick={() => pick(p.bounds)}
                className="w-full rounded-sm px-2 py-1 text-left text-xs hover:bg-muted"
              >
                {p.label}
              </button>
            </li>
          ))}
        </ul>
        <Calendar
          mode="range"
          selected={selected}
          defaultMonth={after ?? now}
          disabled={{ after: now }}
          onSelect={(range) => {
            if (!range?.from) return pick({});
            const next: DateBounds = { after: formatDateValue(range.from) };
            if (range.to) next.before = formatDateValue(addDays(range.to, 1));
            // Stay open: the first click is a one-day range, the second extends it.
            onChange(next);
          }}
        />
      </PopoverContent>
    </Popover>
  );
}
