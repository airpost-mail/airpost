import { FolderInput, ListFilter, Mail, Paperclip, Star, Type, User, X } from 'lucide-react';
import { useRef, type ReactNode } from 'react';
import { Button } from '@/components/ui/button';
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Toggle } from '@/components/ui/toggle';
import { categoryDisplayName, type AccountJSON } from '@/engine';
import { cn } from '@/lib/utils';
import { DateFilter, type DateBounds } from './DateFilter';
import { describeToken } from './operators';
import {
  hasToken,
  toggleToken,
  withToken,
  withoutTerm,
  withoutToken,
  type ParsedSearch,
} from './query';
import { focusSearchBox } from './store';
import { useAllCategories } from './useAllCategories';

const ALL_ACCOUNTS = '__all';
const TOGGLED = new Set(['is:unread', 'is:starred', 'has:attachment']);

function Chip({
  label,
  children,
  invalid,
  onRemove,
  removeLabel,
}: {
  label?: string;
  children: ReactNode;
  invalid?: boolean;
  onRemove: () => void;
  removeLabel: string;
}) {
  return (
    <span
      title={invalid ? "Couldn't read this date" : undefined}
      className={cn(
        'inline-flex h-6 max-w-64 animate-in items-center gap-1 rounded-sm border bg-background pr-0.5 pl-2 text-xs duration-(--duration-base) fade-in-0 zoom-in-95',
        invalid && 'border-destructive/50 text-destructive',
      )}
    >
      {label && <span className="text-muted-foreground">{label}</span>}
      <span className="truncate font-medium">{children}</span>
      <button
        type="button"
        aria-label={removeLabel}
        onClick={onRemove}
        className="ml-0.5 grid size-5 place-items-center rounded-sm text-muted-foreground hover:bg-muted hover:text-foreground"
      >
        <X className="size-3" />
      </button>
    </span>
  );
}

function FlagToggle({
  pressed,
  onToggle,
  icon,
  children,
}: {
  pressed: boolean;
  onToggle: () => void;
  icon: ReactNode;
  children: ReactNode;
}) {
  return (
    <Toggle
      size="sm"
      variant="outline"
      pressed={pressed}
      onPressedChange={onToggle}
      className="h-6 gap-1.5 rounded-md px-2 text-xs font-normal data-[state=on]:border-primary/40 data-[state=on]:bg-primary/10 data-[state=on]:text-primary"
    >
      {icon}
      {children}
    </Toggle>
  );
}

export function SearchFilters({
  parsed,
  onChange,
  accounts,
  accountId,
  onAccountChange,
}: {
  parsed: ParsedSearch;
  onChange: (next: ParsedSearch) => void;
  accounts: AccountJSON[];
  accountId?: string;
  onAccountChange: (accountId: string | undefined) => void;
}) {
  const allCategories = useAllCategories();
  // Picking From/To/Subject moves focus to the search box; don't let the menu take it back.
  const toSearchBox = useRef(false);
  const typeOperator = (op: string) => {
    toSearchBox.current = true;
    focusSearchBox(op);
  };
  const scoped = accountId
    ? allCategories.filter((c) => c.account.id === accountId)
    : allCategories;

  const dateBounds: DateBounds = {
    after: parsed.tokens.find((t) => t.field === 'after')?.value,
    before: parsed.tokens.find((t) => t.field === 'before')?.value,
  };
  const setDates = (b: DateBounds) => {
    let next: ParsedSearch = {
      ...parsed,
      tokens: parsed.tokens.filter((t) => t.field !== 'after' && t.field !== 'before'),
    };
    if (b.after) next = withToken(next, { field: 'after', value: b.after });
    if (b.before) next = withToken(next, { field: 'before', value: b.before });
    onChange(next);
  };

  const flag = (field: 'is' | 'has', value: string) => ({
    pressed: hasToken(parsed, field, value),
    onToggle: () => onChange(toggleToken(parsed, field, value)),
  });

  const chips = parsed.tokens
    .map((t, i) => ({ t, i }))
    // These have their own controls in the row below; unreadable dates still get a chip.
    .filter(({ t }) => {
      if (TOGGLED.has(`${t.field}:${t.value}`)) return false;
      if (t.field === 'after' || t.field === 'before') return !!describeToken(t).invalid;
      return true;
    });

  return (
    <div className="flex flex-col gap-2">
      {(parsed.terms.length > 0 || chips.length > 0) && (
        <div className="flex flex-wrap items-center gap-1.5" aria-label="Active filters">
          {parsed.terms.map((term, i) => (
            <Chip
              key={`term:${i}:${term}`}
              onRemove={() => onChange(withoutTerm(parsed, i))}
              removeLabel={`Remove “${term}”`}
            >
              “{term}”
            </Chip>
          ))}
          {chips.map(({ t, i }) => {
            const d = describeToken(t);
            return (
              <Chip
                key={`token:${i}:${t.field}:${t.value}`}
                label={d.label}
                invalid={d.invalid}
                onRemove={() => onChange(withoutToken(parsed, i))}
                removeLabel={`Remove ${d.label ?? ''} ${d.value}`.replace(/\s+/g, ' ')}
              >
                {d.value}
              </Chip>
            );
          })}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-1.5">
        <FlagToggle {...flag('is', 'unread')} icon={<Mail className="size-3.5" />}>
          Unread
        </FlagToggle>
        <FlagToggle {...flag('is', 'starred')} icon={<Star className="size-3.5" />}>
          Starred
        </FlagToggle>
        <FlagToggle {...flag('has', 'attachment')} icon={<Paperclip className="size-3.5" />}>
          Attachment
        </FlagToggle>
        <DateFilter bounds={dateBounds} onChange={setDates} />

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="sm" className="h-6 gap-1.5 px-2 text-xs">
              <ListFilter className="size-3.5" />
              More
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="start"
            className="min-w-44"
            onCloseAutoFocus={(e) => {
              if (toSearchBox.current) e.preventDefault();
              toSearchBox.current = false;
            }}
          >
            <DropdownMenuItem onSelect={() => typeOperator('from:')}>
              <User /> From…
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => typeOperator('to:')}>
              <User /> To…
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => typeOperator('subject:')}>
              <Type /> Subject…
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <FolderInput /> In folder or label
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent className="max-h-80 overflow-y-auto">
                {scoped.map(({ account, categories }) => (
                  <div key={account.id}>
                    {scoped.length > 1 && (
                      <DropdownMenuLabel className="text-2xs tracking-widest uppercase">
                        {account.label || account.emailAddress}
                      </DropdownMenuLabel>
                    )}
                    {categories.map((c) => {
                      const name = categoryDisplayName(c);
                      return (
                        <DropdownMenuItem
                          key={c.id}
                          onSelect={() =>
                            onChange(withToken(parsed, { field: 'in', value: c.role || name }))
                          }
                        >
                          {name}
                        </DropdownMenuItem>
                      );
                    })}
                  </div>
                ))}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          </DropdownMenuContent>
        </DropdownMenu>

        {accounts.length > 1 && (
          <Select
            value={accountId ?? ALL_ACCOUNTS}
            onValueChange={(v) => onAccountChange(v === ALL_ACCOUNTS ? undefined : v)}
          >
            <SelectTrigger
              size="sm"
              aria-label="Account"
              className="ml-auto h-6! gap-1.5 border-transparent px-2 text-xs shadow-none hover:bg-muted"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent align="end">
              <SelectItem value={ALL_ACCOUNTS}>All accounts</SelectItem>
              {accounts.map((a) => (
                <SelectItem key={a.id} value={a.id}>
                  <span
                    className="size-1.5 rounded-full"
                    style={{ backgroundColor: a.color }}
                    aria-hidden
                  />
                  {a.label || a.emailAddress}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </div>
    </div>
  );
}
