import { Check, Folder, Tag } from 'lucide-react';
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command';
import {
  categoryDisplayName,
  isGmailAccount,
  isStandardCategory,
  threadInCategory,
  useAccount,
  useCategories,
  type CategoryJSON,
  type LabelJSON,
} from '@/engine';
import { useShortcutScope } from '@/lib/shortcuts';
import { useThreadActions } from './actions';
import { LabelDot } from './LabelChip';
import { closeCategoryPicker, useCategoryPicker, type PickerRequest } from './picker-store';

// On Gmail, Spam and Trash are the only real move targets; everything else is a label.
const GMAIL_MOVE_ROLES = new Set(['spam', 'trash']);

function moveTargets(cats: CategoryJSON[], gmail: boolean) {
  return cats.filter((c) => {
    if (gmail)
      return c.__cls === 'Label'
        ? c.role === '' || c.role === 'inbox'
        : GMAIL_MOVE_ROLES.has(c.role);
    return c.__cls === 'Folder' && c.role !== 'sent' && c.role !== 'drafts';
  });
}

function Picker({ req }: { req: PickerRequest }) {
  const accountId = req.threads[0].aid;
  const account = useAccount(accountId);
  const { all, labels } = useCategories(accountId);
  const actions = useThreadActions();
  const gmail = account ? isGmailAccount(account) : false;
  const sameAccount = req.threads.every((t) => t.aid === accountId);
  const targets =
    req.kind === 'move' ? moveTargets(all, gmail) : labels.filter((l) => !isStandardCategory(l));
  const n = req.threads.length;
  const title = req.kind === 'move' ? 'Move to' : 'Label';

  const pick = (c: CategoryJSON) => {
    closeCategoryPicker();
    if (req.kind === 'move') actions.moveTo(req.threads, c, req.viewing);
    else actions.toggleLabel(req.threads, c as LabelJSON);
  };

  return (
    <Command loop>
      <div className="flex items-baseline justify-between px-3 pt-3 pb-1">
        <span className="text-xl font-semibold tracking-tight">{title}</span>
        <span className="font-mono text-2xs text-muted-foreground tabular">
          {n === 1 ? '1 thread' : `${n} threads`}
        </span>
      </div>
      <CommandInput placeholder={req.kind === 'move' ? 'Folder name' : 'Label name'} />
      <CommandList>
        {!sameAccount ? (
          <p className="px-3 py-6 text-center text-sm text-muted-foreground">
            Select threads from one account to {req.kind === 'move' ? 'move' : 'label'} them.
          </p>
        ) : (
          <>
            <CommandEmpty>Nothing matches.</CommandEmpty>
            <CommandGroup>
              {targets.map((c) => {
                const name = categoryDisplayName(c);
                const applied = req.threads.every((t) => threadInCategory(t, c.id));
                return (
                  <CommandItem key={c.id} value={`${name} ${c.id}`} onSelect={() => pick(c)}>
                    {c.__cls === 'Label' && !isStandardCategory(c) ? (
                      <LabelDot label={c} className="mx-[5px]" />
                    ) : c.__cls === 'Label' ? (
                      <Tag className="text-muted-foreground" />
                    ) : (
                      <Folder className="text-muted-foreground" />
                    )}
                    <span className="truncate">{name}</span>
                    {applied && <Check className="ml-auto text-primary" aria-label="Applied" />}
                  </CommandItem>
                );
              })}
            </CommandGroup>
          </>
        )}
      </CommandList>
    </Command>
  );
}

/** Mounted once by the thread list. */
export function CategoryPicker() {
  const req = useCategoryPicker();
  useShortcutScope('picker', !!req);
  return (
    <CommandDialog
      open={!!req}
      onOpenChange={(open) => !open && closeCategoryPicker()}
      title={req?.kind === 'label' ? 'Label threads' : 'Move threads'}
      description="Pick a destination"
      className="sm:max-w-sm"
    >
      {req && <Picker req={req} />}
    </CommandDialog>
  );
}
