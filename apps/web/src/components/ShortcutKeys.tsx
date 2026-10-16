import { Kbd, KbdGroup } from '@/components/ui/kbd';
import { formatShortcut } from '@/lib/shortcuts';
import { cn } from '@/lib/utils';

/** Renders one binding (`mod+k`, `g i`) as key caps. */
export function ShortcutKeys({ keys, className }: { keys: string; className?: string }) {
  return (
    <KbdGroup className={cn('font-mono', className)}>
      {formatShortcut(keys).map((k, i) =>
        k === 'then' ? (
          <span key={i} className="text-2xs text-muted-foreground">
            then
          </span>
        ) : (
          <Kbd key={i} className="font-mono text-2xs">
            {k}
          </Kbd>
        ),
      )}
    </KbdGroup>
  );
}
