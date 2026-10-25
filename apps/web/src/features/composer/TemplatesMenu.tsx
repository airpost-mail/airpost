import { FileText } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { useEngine } from '@/engine';
import { loadTemplate, useTemplates } from './templates';

export function TemplatesMenu({
  onInsert,
}: {
  onInsert: (t: { subject: string; body: string }) => void;
}) {
  const client = useEngine();
  const templates = useTemplates();
  const list = templates.data ?? [];

  const insert = (name: string) =>
    loadTemplate(client, name).then(onInsert, (err: unknown) =>
      toast.error(`Couldn't load the template: ${err instanceof Error ? err.message : err}`),
    );

  return (
    // Another client may have changed them since the last look.
    <DropdownMenu onOpenChange={(open) => open && void templates.refetch()}>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              size="icon-xs"
              variant="ghost"
              aria-label="Templates"
              className="text-muted-foreground hover:text-foreground"
            >
              <FileText aria-hidden />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>Templates</TooltipContent>
      </Tooltip>
      <DropdownMenuContent side="top" align="start" className="w-56">
        <DropdownMenuLabel className="text-2xs tracking-widest text-muted-foreground uppercase">
          Templates
        </DropdownMenuLabel>
        {list.length === 0 && (
          <p className="px-2 py-1.5 text-xs text-muted-foreground">
            {templates.isError
              ? "Couldn't read the templates."
              : 'No templates yet. Add them in the desktop app.'}
          </p>
        )}
        {list.map((t) => (
          <DropdownMenuItem key={t.id} onSelect={() => void insert(t.name)}>
            <span className="truncate">{t.name}</span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
