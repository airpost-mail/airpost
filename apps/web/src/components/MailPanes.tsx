import type { ReactNode } from 'react';
import { useDefaultLayout } from 'react-resizable-panels';
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/components/ui/resizable';
import { usePreferences } from '@/lib/preferences';
import { cn } from '@/lib/utils';

/**
 * Thread list and reading pane, placed by the `readingPane` preference. Sizes persist per
 * browser and per orientation. With the pane off, an open thread replaces the list; the list
 * stays mounted (hidden) so its keyboard shortcuts keep working.
 */
export function MailPanes({
  list,
  reader,
  open,
}: {
  list: ReactNode;
  reader: ReactNode;
  /** A thread is open in `reader`. */
  open: boolean;
}) {
  const { readingPane } = usePreferences();
  if (readingPane === 'off') {
    return (
      <>
        <ListSection className={cn(open && 'hidden')}>{list}</ListSection>
        {open && <ReaderSection>{reader}</ReaderSection>}
      </>
    );
  }
  return (
    <SplitPanes key={readingPane} list={list} reader={reader} bottom={readingPane === 'bottom'} />
  );
}

function SplitPanes({
  list,
  reader,
  bottom,
}: {
  list: ReactNode;
  reader: ReactNode;
  bottom: boolean;
}) {
  const { defaultLayout, onLayoutChanged } = useDefaultLayout({
    id: bottom ? 'airpost-mail-panes-bottom' : 'airpost-mail-panes',
    storage: localStorage,
  });
  return (
    <ResizablePanelGroup
      orientation={bottom ? 'vertical' : 'horizontal'}
      defaultLayout={defaultLayout}
      onLayoutChanged={onLayoutChanged}
    >
      <ResizablePanel id="list" defaultSize={bottom ? '45' : '40'} minSize={bottom ? 160 : 300}>
        <ListSection>{list}</ListSection>
      </ResizablePanel>
      <ResizableHandle />
      <ResizablePanel id="reader" minSize="30">
        <ReaderSection>{reader}</ReaderSection>
      </ResizablePanel>
    </ResizablePanelGroup>
  );
}

function ListSection({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <section aria-label="Threads" className={cn('h-full min-w-0 overflow-hidden', className)}>
      {children}
    </section>
  );
}

function ReaderSection({ children }: { children: ReactNode }) {
  return (
    <section aria-label="Reading pane" className="h-full min-w-0 overflow-hidden bg-reader">
      {children}
    </section>
  );
}
