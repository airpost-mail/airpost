import { Link, Outlet, useNavigate } from '@tanstack/react-router';
import {
  AtSign,
  Keyboard,
  Layers,
  PenLine,
  Plug,
  SlidersHorizontal,
  type LucideIcon,
} from 'lucide-react';
import { ScrollArea } from '@/components/ui/scroll-area';
import { useShortcut } from '@/lib/shortcuts';

interface NavItem {
  to:
    | '/settings'
    | '/settings/accounts'
    | '/settings/signatures'
    | '/settings/shortcuts'
    | '/settings/integrations'
    | '/settings/capabilities';
  label: string;
  icon: LucideIcon;
}

const NAV: NavItem[] = [
  { to: '/settings', label: 'General', icon: SlidersHorizontal },
  { to: '/settings/accounts', label: 'Accounts', icon: AtSign },
  { to: '/settings/signatures', label: 'Signatures', icon: PenLine },
  { to: '/settings/shortcuts', label: 'Shortcuts', icon: Keyboard },
  { to: '/settings/integrations', label: 'Integrations', icon: Plug },
  { to: '/settings/capabilities', label: 'Capabilities', icon: Layers },
];

export function SettingsLayout() {
  const navigate = useNavigate();
  useShortcut('escape', () => void navigate({ to: '/' }), {
    description: 'Close settings',
    group: 'Navigation',
  });

  return (
    <div className="flex h-full min-h-0">
      <nav
        aria-label="Settings"
        className="flex w-52 shrink-0 flex-col gap-4 border-r border-hairline px-3 py-6"
      >
        <p className="px-2 text-2xl font-semibold tracking-tight">Settings</p>
        <ul className="flex flex-col gap-px">
          {NAV.map(({ to, label, icon: Icon }) => (
            <li key={to}>
              <Link
                to={to}
                activeOptions={{ exact: to === '/settings', includeSearch: false }}
                className="relative flex h-8 items-center gap-2.5 rounded-md px-2 text-sm text-muted-foreground transition-colors duration-(--duration-fast) hover:bg-sidebar-accent hover:text-foreground data-[status=active]:bg-sidebar-accent data-[status=active]:text-foreground data-[status=active]:before:absolute data-[status=active]:before:inset-y-1.5 data-[status=active]:before:-left-3 data-[status=active]:before:w-0.5 data-[status=active]:before:rounded-full data-[status=active]:before:bg-primary"
              >
                <Icon className="size-4" />
                {label}
              </Link>
            </li>
          ))}
        </ul>
      </nav>
      <ScrollArea className="min-w-0 flex-1">
        <div className="mx-auto max-w-2xl px-8 py-10">
          <Outlet />
        </div>
      </ScrollArea>
    </div>
  );
}
