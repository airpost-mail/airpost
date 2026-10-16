import { LogOut } from 'lucide-react';
import { Avatar, AvatarFallback } from '@/components/ui/avatar';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { contactInitials } from '@/engine';
import { serverLabel } from './server-url';
import { useSession } from './session';
import { useSignOut } from './use-sign-out';

/** Avatar button for the top bar: who is signed in, to which server, and Sign out. */
export function SessionMenu() {
  const session = useSession();
  const signOut = useSignOut();
  if (!session) return null;
  const { user, server, method } = session;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label={`Signed in as ${user.email}`}>
          <Avatar size="sm" className="size-5">
            <AvatarFallback className="bg-primary/12 text-[10px] font-medium text-primary">
              {contactInitials({ name: user.name, email: user.email })}
            </AvatarFallback>
          </Avatar>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <DropdownMenuLabel className="flex flex-col gap-0.5 py-1.5">
          <span className="truncate text-sm text-foreground">{user.name || user.email}</span>
          <span className="truncate font-normal">{user.email}</span>
        </DropdownMenuLabel>
        <DropdownMenuLabel className="flex items-center justify-between gap-2 pt-0 font-normal">
          <span className="text-2xs tracking-widest uppercase">
            {method === 'demo' ? 'Demo' : 'Server'}
          </span>
          <span className="truncate font-mono text-2xs tabular">
            {method === 'demo' ? 'in-browser engine' : serverLabel(server.url)}
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
