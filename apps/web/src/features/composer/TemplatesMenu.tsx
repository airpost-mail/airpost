import { FileText, Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Input } from '@/components/ui/input';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { useShortcutScope } from '@/lib/shortcuts';
import { isBlankHtml } from './html';
import { deleteTemplate, saveTemplate, useTemplates, type Template } from './templates';

export function TemplatesMenu({
  current,
  onInsert,
}: {
  current: { subject: string; body: string };
  onInsert: (t: Template) => void;
}) {
  const templates = useTemplates();
  const [saving, setSaving] = useState(false);

  return (
    <>
      <DropdownMenu>
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
          {templates.length === 0 && (
            <p className="px-2 py-1.5 text-xs text-muted-foreground">No templates yet.</p>
          )}
          {templates.map((t) => (
            <DropdownMenuItem key={t.id} onSelect={() => onInsert(t)}>
              <span className="truncate">{t.name}</span>
            </DropdownMenuItem>
          ))}
          <DropdownMenuSeparator />
          <DropdownMenuItem disabled={isBlankHtml(current.body)} onSelect={() => setSaving(true)}>
            <Plus aria-hidden />
            Save as template…
          </DropdownMenuItem>
          {templates.length > 0 && (
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <Trash2 aria-hidden />
                Delete a template
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                {templates.map((t) => (
                  <DropdownMenuItem
                    key={t.id}
                    variant="destructive"
                    onSelect={() => deleteTemplate(t.id)}
                  >
                    {t.name}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      <SaveTemplateDialog
        open={saving}
        onOpenChange={setSaving}
        defaultName={current.subject}
        onSave={(name) => {
          saveTemplate({ name, subject: current.subject, body: current.body });
          setSaving(false);
          toast.success(`Saved template “${name}”`);
        }}
      />
    </>
  );
}

function SaveTemplateDialog({
  open,
  onOpenChange,
  defaultName,
  onSave,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  defaultName: string;
  onSave: (name: string) => void;
}) {
  useShortcutScope('composer-dialog', open);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle className="text-xl font-semibold tracking-tight">New template</DialogTitle>
          <DialogDescription>
            Saves this message's subject and text in this browser.
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            const name = String(new FormData(e.currentTarget).get('name') ?? '').trim();
            if (name) onSave(name);
          }}
        >
          <Input
            name="name"
            autoFocus
            defaultValue={defaultName}
            aria-label="Template name"
            required
          />
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit">Save</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
