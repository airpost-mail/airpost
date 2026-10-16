import type { LucideIcon } from 'lucide-react';
import { ShortcutKeys } from '@/components/ShortcutKeys';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';

/** Icon-only toolbar button; the tooltip names the action and its shortcut. */
export function ActionButton({
  icon: Icon,
  label,
  keys,
  onClick,
  disabled,
  active,
  className,
}: {
  icon: LucideIcon;
  label: string;
  keys?: string;
  onClick: () => void;
  disabled?: boolean;
  /** Pressed state, e.g. a starred thread. */
  active?: boolean;
  className?: string;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={label}
          aria-pressed={active}
          disabled={disabled}
          onClick={onClick}
          className={cn('text-muted-foreground hover:text-foreground', className)}
        >
          <Icon className="size-4" />
        </Button>
      </TooltipTrigger>
      <TooltipContent className="flex items-center gap-2">
        {label}
        {keys && <ShortcutKeys keys={keys} />}
      </TooltipContent>
    </Tooltip>
  );
}
