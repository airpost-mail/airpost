import { toast } from 'sonner';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { useQueueTask, type ThreadJSON } from '@/engine';
import { cn } from '@/lib/utils';
import { useTriageShown } from './hooks';
import {
  CATEGORIES,
  CATEGORY_LABEL,
  chipCategory,
  correctionTask,
  triageOf,
  type TriageCategory,
} from './triage';

const TONE = {
  'needs-reply': 'bg-warning/20 text-foreground',
  important: 'bg-destructive/10 text-destructive',
};

/**
 * The triage chip (needs-reply and important only) with a menu to correct the category.
 * Nothing shows unless the engine triages the thread's account.
 */
export function TriageChip({ thread, className }: { thread: ThreadJSON; className?: string }) {
  const shown = useTriageShown(thread.aid);
  const { mutate } = useQueueTask();
  const value = triageOf(thread);
  const category = shown ? chipCategory(value, thread) : null;
  if (!value || !category) return null;

  const correct = (next: string) => {
    if (next === value.category && value.source === 'user') return;
    mutate(correctionTask(thread, next as TriageCategory), {
      onError: () => toast.error("Couldn't reach the engine. The category is unchanged."),
    });
  };
  const title =
    value.source === 'user' ? 'You filed this. Click to change.' : 'Sorted by AI. Click to change.';
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        title={title}
        aria-label={`${CATEGORY_LABEL[category]}: change category`}
        className={cn(
          'inline-flex h-4.5 shrink-0 items-center rounded-sm px-1.5 text-2xs font-medium outline-offset-1',
          TONE[category],
          className,
        )}
      >
        {CATEGORY_LABEL[category]}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
          {value.reason && value.source !== 'user' ? value.reason : 'Category'}
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuRadioGroup value={value.category} onValueChange={correct}>
          {CATEGORIES.map((c) => (
            <DropdownMenuRadioItem key={c} value={c}>
              {CATEGORY_LABEL[c]}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
