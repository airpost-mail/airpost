import { useEffect, type ReactNode } from 'react';
import { useDefaultLayout } from 'react-resizable-panels';
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/components/ui/resizable';
import { TooltipProvider } from '@/components/ui/tooltip';
import { installShortcuts, useShortcut } from '@/lib/shortcuts';
import { setShortcutsSheetOpen, useShortcutsSheetOpen } from './shortcuts-sheet';
import { ShortcutsSheet } from './ShortcutsSheet';
import { TopBar } from './TopBar';

export interface AppShellProps {
  /** Left pane (the mailbox feature's account and folder list). Omit for full-width pages. */
  sidebar?: ReactNode;
  /** Centre of the top bar (the search feature's trigger). */
  search?: ReactNode;
  /** Right side of the top bar, before the app buttons (connection status). */
  status?: ReactNode;
  /** Last item of the top bar (the signed-in user's menu). */
  userMenu?: ReactNode;
  /** A strip between the top bar and the panes (offline / reconnecting). */
  banner?: ReactNode;
  /** Floating layers that need the shell's providers (composer dock, palette). */
  overlays?: ReactNode;
  /** The routed content; mail routes split it further with <MailPanes>. */
  children: ReactNode;
}

export function AppShell({
  sidebar,
  search,
  status,
  userMenu,
  banner,
  overlays,
  children,
}: AppShellProps) {
  const shortcutsOpen = useShortcutsSheetOpen();
  const { defaultLayout, onLayoutChanged } = useDefaultLayout({
    id: 'airpost-shell',
    storage: localStorage,
  });

  useEffect(() => installShortcuts(window), []);
  useShortcut('?', () => setShortcutsSheetOpen(!shortcutsOpen), {
    description: 'Show keyboard shortcuts',
    group: 'Application',
    scope: '*',
  });

  return (
    <TooltipProvider delayDuration={500}>
      <div className="flex h-dvh flex-col overflow-hidden bg-background text-foreground">
        <TopBar
          search={search}
          status={status}
          userMenu={userMenu}
          onShowShortcuts={() => setShortcutsSheetOpen(true)}
        />
        {banner}
        {sidebar ? (
          <ResizablePanelGroup
            orientation="horizontal"
            defaultLayout={defaultLayout}
            onLayoutChanged={onLayoutChanged}
            className="min-h-0 flex-1"
          >
            <ResizablePanel id="sidebar" defaultSize="17" minSize={184} maxSize={360} collapsible>
              <nav
                aria-label="Mailboxes"
                className="h-full overflow-hidden bg-sidebar text-sidebar-foreground"
              >
                {sidebar}
              </nav>
            </ResizablePanel>
            <ResizableHandle />
            <ResizablePanel id="main" minSize="40">
              <main className="h-full min-w-0 overflow-hidden">{children}</main>
            </ResizablePanel>
          </ResizablePanelGroup>
        ) : (
          <main className="min-h-0 min-w-0 flex-1 overflow-hidden">{children}</main>
        )}
        <ShortcutsSheet open={shortcutsOpen} onOpenChange={setShortcutsSheetOpen} />
      </div>
      {overlays}
    </TooltipProvider>
  );
}
