import { LogOut, MonitorSmartphone } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { isAdmin, useSession } from './session';
import { useSignOut } from './use-sign-out';

/** Button for the top bar: this browser's name and access, where it's paired, and Sign out. */
export function SessionMenu() {
  const session = useSession();
  const signOut = useSignOut();
  if (!session) return null;
  const { device, demo } = session;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label={`Paired as ${device.name}`}>
          <MonitorSmartphone />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <DropdownMenuLabel className="flex flex-col gap-0.5 py-1.5">
          <span className="truncate text-sm text-foreground">{device.name}</span>
          <span className="truncate font-normal">
            {isAdmin(session) ? 'Admin access' : 'Mail access'}
          </span>
        </DropdownMenuLabel>
        <DropdownMenuLabel className="flex items-center justify-between gap-2 pt-0 font-normal">
          <span className="text-2xs tracking-widest uppercase">{demo ? 'Demo' : 'Gateway'}</span>
          <span className="truncate font-mono text-2xs tabular">
            {demo ? 'in-browser engine' : window.location.host}
          </span>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => void signOut()}>
          <LogOut /> Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
