import { useNavigate } from '@tanstack/react-router';
import {
  AtSign,
  FolderClosed,
  Inbox,
  Keyboard,
  type LucideIcon,
  Moon,
  PenLine,
  RefreshCw,
  Rows3,
  Search,
  Settings,
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { ShortcutKeys } from '@/components/ShortcutKeys';
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from '@/components/ui/command';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Kbd } from '@/components/ui/kbd';
import {
  useAccounts,
  useSearch,
  useSearchGrammar,
  type AccountJSON,
  type ThreadJSON,
} from '@/engine';
import { formatListDate } from '@/lib/format';
import { useShortcut, useShortcutScope } from '@/lib/shortcuts';
import { cn } from '@/lib/utils';
import { filterCommands } from './palette';
import { toEngineQuery } from './query';
import { addRecentSearch } from './recent';
import { setCommandPaletteOpen, useCommandPaletteOpen } from './store';
import { threadSender } from './thread-sender';
import { usePaletteCommands, type IconKey, type PaletteEntry } from './usePaletteCommands';

const ICONS: Record<IconKey, LucideIcon> = {
  compose: PenLine,
  theme: Moon,
  density: Rows3,
  settings: Settings,
  shortcuts: Keyboard,
  sync: RefreshCw,
  account: AtSign,
  inbox: Inbox,
  category: FolderClosed,
};

const MAIL_LIMIT = 6;

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

/** cmd+k: jump anywhere, run any action, or search mail. Mounted once by the shell. */
export function CommandPalette() {
  const open = useCommandPaletteOpen();
  const [query, setQuery] = useState('');

  useShortcut('mod+k', () => setCommandPaletteOpen(!open), {
    description: 'Command palette',
    group: 'Application',
    allowInInputs: true,
    scope: '*',
  });
  useShortcutScope('palette', open);

  // After an action, focus belongs to whatever the action opened, not the old element.
  const ranAction = useRef(false);

  const onOpenChange = (next: boolean) => {
    setCommandPaletteOpen(next);
    if (!next) setQuery('');
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showCloseButton={false}
        className="top-1/4 translate-y-0 overflow-hidden rounded-xl! p-0 sm:max-w-xl"
        onCloseAutoFocus={(e) => {
          if (ranAction.current) e.preventDefault();
          ranAction.current = false;
        }}
      >
        <DialogHeader className="sr-only">
          <DialogTitle>Command palette</DialogTitle>
          <DialogDescription>Jump to a mailbox, run a command or search mail</DialogDescription>
        </DialogHeader>
        {/* Mounted only while open so the engine is not searched in the background. */}
        {open && (
          <PaletteBody
            query={query}
            setQuery={setQuery}
            close={() => {
              ranAction.current = true;
              onOpenChange(false);
            }}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

function PaletteBody({
  query,
  setQuery,
  close,
}: {
  query: string;
  setQuery: (q: string) => void;
  close: () => void;
}) {
  const navigate = useNavigate();
  const commands = usePaletteCommands();
  const matches = filterCommands(commands, query);
  const debounced = useDebounced(query.trim(), 120);
  const grammar = useSearchGrammar();
  const engineQuery = debounced.length >= 2 ? toEngineQuery(debounced, grammar) : '';
  const mail = useSearch(engineQuery ? { query: engineQuery, grammar } : null, {
    limit: MAIL_LIMIT,
  });
  const { data: accounts = [] } = useAccounts();
  const trimmed = query.trim();

  const run = (fn: () => void) => {
    close();
    // Let the dialog hand focus back before the action moves it (sheet, composer).
    requestAnimationFrame(fn);
  };
  const searchAll = () => {
    addRecentSearch(trimmed);
    void navigate({ to: '/search', search: { q: trimmed } });
  };
  const openResult = (t: ThreadJSON) => {
    addRecentSearch(trimmed);
    void navigate({
      to: '/search',
      search: { q: trimmed, thread: t.id, threadAccount: t.aid },
    });
  };

  const actions = matches.filter((c) => c.group === 'Actions');
  const places = matches.filter((c) => c.group === 'Go to');
  // A strong command match ("set" -> Settings) goes above the mail results.
  const commandsFirst =
    !trimmed || (matches[0] && matches[0].label.toLowerCase().startsWith(trimmed.toLowerCase()));
  const results = engineQuery && trimmed ? (mail.data?.threads ?? []) : [];

  const mailGroup = trimmed && (
    <CommandGroup heading="Mail" key="mail">
      <CommandItem value="search-all" onSelect={() => run(searchAll)}>
        <Search />
        <span className="truncate">
          Search mail for <span className="font-medium">“{trimmed}”</span>
        </span>
        <CommandShortcut>
          <Kbd className="font-mono text-2xs">↵</Kbd>
        </CommandShortcut>
      </CommandItem>
      {results.map((t) => (
        <MailResult
          key={`${t.aid}:${t.id}`}
          thread={t}
          account={accounts.find((a) => a.id === t.aid)}
          showAccount={accounts.length > 1}
          onSelect={() => run(() => openResult(t))}
        />
      ))}
    </CommandGroup>
  );
  const commandGroups = [
    actions.length > 0 && (
      <CommandGroup heading="Actions" key="actions">
        {actions.map((c) => (
          <CommandRow key={c.id} entry={c} onSelect={() => run(c.run)} />
        ))}
      </CommandGroup>
    ),
    places.length > 0 && (
      <CommandGroup heading="Go to" key="places">
        {places.map((c) => (
          <CommandRow key={c.id} entry={c} onSelect={() => run(c.run)} />
        ))}
      </CommandGroup>
    ),
  ];

  return (
    <Command shouldFilter={false} loop className="rounded-none! bg-transparent p-0">
      <CommandInput
        placeholder="Search mail or type a command"
        value={query}
        onValueChange={setQuery}
        className="h-full"
      />
      <CommandList className="max-h-[min(26rem,60vh)] px-1 pb-1">
        <CommandEmpty>
          <span className="text-lg font-semibold tracking-tight">Nothing on the chart.</span>
        </CommandEmpty>
        {commandsFirst ? [...commandGroups, mailGroup] : [mailGroup, ...commandGroups]}
      </CommandList>
      <footer className="flex items-center gap-4 border-t border-hairline px-3 py-2 text-2xs text-muted-foreground">
        <span className="flex items-center gap-1">
          <Kbd className="font-mono text-2xs">↑</Kbd>
          <Kbd className="font-mono text-2xs">↓</Kbd> move
        </span>
        <span className="flex items-center gap-1">
          <Kbd className="font-mono text-2xs">↵</Kbd> open
        </span>
        <span className="flex items-center gap-1">
          <Kbd className="font-mono text-2xs">Esc</Kbd> close
        </span>
        {mail.isFetching && <span className="ml-auto font-mono">searching…</span>}
      </footer>
    </Command>
  );
}

function CommandRow({ entry, onSelect }: { entry: PaletteEntry; onSelect: () => void }) {
  const Icon = ICONS[entry.icon];
  return (
    <CommandItem value={entry.id} onSelect={onSelect}>
      {entry.color ? (
        <span className="grid size-4 place-items-center" aria-hidden>
          <span className="size-1.5 rounded-full" style={{ backgroundColor: entry.color }} />
        </span>
      ) : (
        <Icon className="text-muted-foreground" />
      )}
      <span className="truncate">{entry.label}</span>
      {entry.detail && (
        <span className="truncate text-xs text-muted-foreground">{entry.detail}</span>
      )}
      {entry.shortcut && (
        <CommandShortcut>
          <ShortcutKeys keys={entry.shortcut} />
        </CommandShortcut>
      )}
    </CommandItem>
  );
}

function MailResult({
  thread: t,
  account,
  showAccount,
  onSelect,
}: {
  thread: ThreadJSON;
  account?: AccountJSON;
  showAccount: boolean;
  onSelect: () => void;
}) {
  const unread = t.unread > 0;
  return (
    <CommandItem value={`thread:${t.aid}:${t.id}`} onSelect={onSelect} className="gap-2.5">
      <span className="grid size-4 shrink-0 place-items-center" aria-hidden>
        <span className={cn('size-1.5 rounded-full', unread && 'bg-unread')} />
      </span>
      <span className="flex w-32 shrink-0 items-center gap-1.5 text-muted-foreground">
        {showAccount && account && (
          <span
            className="size-1.5 shrink-0 rounded-full"
            style={{ backgroundColor: account.color }}
            aria-hidden
          />
        )}
        <span className="truncate">{threadSender(t, account)}</span>
      </span>
      <span className={cn('truncate', unread && 'font-semibold')}>
        {t.subject || '(no subject)'}
      </span>
      <CommandShortcut className="tracking-normal">
        <span className="font-mono text-2xs tabular">{formatListDate(t.lmt)}</span>
      </CommandShortcut>
    </CommandItem>
  );
}
