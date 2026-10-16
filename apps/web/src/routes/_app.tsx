// The authenticated shell layout.
import { createFileRoute, Outlet } from '@tanstack/react-router';
import { AppShell } from '@/components/AppShell';
import {
  ConnectionStatus,
  OfflineBanner,
  requireSession,
  SessionGuard,
  SessionMenu,
} from '@/features/auth';
import { ComposerHost } from '@/features/composer';
import { MailboxSidebar } from '@/features/mailbox';
import { CommandPalette, SearchBox } from '@/features/search';

export const Route = createFileRoute('/_app')({
  beforeLoad: ({ location }) => requireSession({ location }),
  component: AppLayout,
});

function AppLayout() {
  return (
    <AppShell
      sidebar={<MailboxSidebar />}
      search={<SearchBox />}
      status={<ConnectionStatus />}
      userMenu={<SessionMenu />}
      banner={<OfflineBanner />}
      overlays={
        <>
          <ComposerHost />
          <CommandPalette />
          <SessionGuard />
        </>
      }
    >
      <Outlet />
    </AppShell>
  );
}
