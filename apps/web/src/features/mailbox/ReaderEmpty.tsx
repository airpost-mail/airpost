import { EmptyState } from '@/components/EmptyState';
import { ShortcutKeys } from '@/components/ShortcutKeys';

export function ReaderEmpty() {
  return (
    <EmptyState
      title="Quiet waters"
      description={
        <span className="inline-flex items-center gap-1.5">
          Pick a thread, or move with <ShortcutKeys keys="j" /> <ShortcutKeys keys="k" />
        </span>
      }
    />
  );
}
