import { ChevronRight } from 'lucide-react';
import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';
import { useCollapsed } from './collapsed';

/** A titled, collapsible group of sidebar rows. Untitled sections can't collapse. */
export function NavSection({
  id,
  title,
  children,
}: {
  id: string;
  title?: string;
  children: ReactNode;
}) {
  const [collapsed, toggle] = useCollapsed(id);
  return (
    <section className="flex flex-col gap-px px-2 py-2">
      {title && (
        <button
          type="button"
          onClick={toggle}
          aria-expanded={!collapsed}
          className="group/section flex h-6 items-center gap-1 rounded-sm px-2 text-2xs font-medium tracking-widest text-muted-foreground uppercase hover:text-foreground"
        >
          {title}
          <ChevronRight
            aria-hidden
            className={cn(
              'size-3 opacity-0 transition-transform duration-(--duration-fast) group-hover/section:opacity-100 group-focus-visible/section:opacity-100',
              !collapsed && 'rotate-90',
            )}
          />
        </button>
      )}
      {!(title && collapsed) && children}
    </section>
  );
}
