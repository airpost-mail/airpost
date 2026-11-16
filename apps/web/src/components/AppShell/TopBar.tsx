import { Link } from '@tanstack/react-router';
import { Keyboard, Settings } from 'lucide-react';
import type { ReactNode } from 'react';
import { Wordmark } from '@/components/Logo';
import { ThemeToggle } from '@/components/ThemeToggle';
import { Button } from '@/components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';

export function TopBar({
  search,
  status,
  userMenu,
  onShowShortcuts,
}: {
  search?: ReactNode;
  status?: ReactNode;
  userMenu?: ReactNode;
  onShowShortcuts: () => void;
}) {
  return (
    <header className="relative flex h-11 shrink-0 items-center gap-3 border-b bg-topbar px-3">
      <Link to="/" className="rounded-sm outline-offset-4" aria-label="Airpost, go to inbox">
        <Wordmark />
      </Link>
      <div className="mx-auto flex w-full max-w-xl min-w-0 justify-center">{search}</div>
      <div className="flex items-center gap-0.5">
        {status}
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Keyboard shortcuts"
              onClick={onShowShortcuts}
            >
              <Keyboard className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Keyboard shortcuts (?)</TooltipContent>
        </Tooltip>
        <ThemeToggle />
        <Button variant="ghost" size="icon-sm" aria-label="Settings" asChild>
          <Link to="/settings">
            <Settings className="size-4" />
          </Link>
        </Button>
        {userMenu}
      </div>
      {/* A cobalt hairline: the one flourish in the chrome. */}
      <span className="pointer-events-none absolute inset-x-0 -bottom-px h-px bg-gradient-to-r from-transparent via-primary/40 to-transparent" />
    </header>
  );
}
