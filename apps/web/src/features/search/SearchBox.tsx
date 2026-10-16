import { useMatch, useNavigate, useParams } from '@tanstack/react-router';
import { History, Search, X } from 'lucide-react';
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { ShortcutKeys } from '@/components/ShortcutKeys';
import { Kbd } from '@/components/ui/kbd';
import { useContacts } from '@/engine';
import { useShortcut } from '@/lib/shortcuts';
import { cn } from '@/lib/utils';
import { activeOperator } from './query';
import { addRecentSearch, useRecentSearches } from './recent';
import { registerSearchBox } from './store';
import { suggest, type Suggestion } from './suggest';
import { useAllCategories } from './useAllCategories';

/** The top bar search input. `/` focuses it; Enter opens the results page. */
export function SearchBox({ className }: { className?: string }) {
  const navigate = useNavigate();
  const searchMatch = useMatch({ from: '/_app/search', shouldThrow: false });
  const { accountId: routeAccountId } = useParams({ strict: false });
  const routeQ = searchMatch?.search.q ?? '';
  const searchAccount = searchMatch?.search.account;

  // Follow the URL (chip edits, back/forward) without an effect.
  const [synced, setSynced] = useState(routeQ);
  const [draft, setDraft] = useState(routeQ);
  if (synced !== routeQ) {
    setSynced(routeQ);
    setDraft(routeQ);
  }

  const [focused, setFocused] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const [active, setActive] = useState(-1);
  const inputRef = useRef<HTMLInputElement>(null);
  const listId = useId();

  const op = activeOperator(draft);
  const contactQuery = op && (op.field === 'from' || op.field === 'to') ? op.value : '';
  const { data: contacts = [] } = useContacts(contactQuery, {
    accountId: searchAccount ?? routeAccountId,
    limit: 6,
  });
  const allCategories = useAllCategories();
  const recent = useRecentSearches();

  const suggestions = useMemo(
    () =>
      suggest(draft, {
        contacts: contactQuery ? contacts : [],
        categories: allCategories.flatMap(({ account, categories }) =>
          categories.map((category) => ({
            category,
            accountName: account.label || account.emailAddress,
          })),
        ),
        recent,
        multipleAccounts: allCategories.length > 1,
      }),
    [draft, contactQuery, contacts, allCategories, recent],
  );
  const open = focused && !dismissed && suggestions.length > 0;

  const focusEnd = () => {
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    });
  };

  useEffect(
    () =>
      registerSearchBox((insert) => {
        if (insert) setDraft((d) => (d && !d.endsWith(' ') ? `${d} ` : d) + insert);
        setDismissed(false);
        focusEnd();
      }),
    [],
  );

  useShortcut('/', () => focusEnd(), { description: 'Search', group: 'Search' });

  const edit = (value: string) => {
    setDraft(value);
    setActive(-1);
    setDismissed(false);
  };

  const submit = (value: string) => {
    const q = value.trim();
    if (q) addRecentSearch(q);
    setDismissed(true);
    setActive(-1);
    void navigate({
      to: '/search',
      search: { q: q || undefined, account: searchAccount },
    });
    inputRef.current?.blur();
  };

  const accept = (s: Suggestion) => {
    if (s.submit) {
      setDraft(s.value);
      submit(s.value);
      return;
    }
    edit(s.value);
    focusEnd();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown' && open) {
      e.preventDefault();
      setActive((i) => (i + 1) % suggestions.length);
    } else if (e.key === 'ArrowUp' && open) {
      e.preventDefault();
      setActive((i) => (i <= 0 ? suggestions.length - 1 : i - 1));
    } else if (e.key === 'Tab' && open && op && !e.shiftKey) {
      e.preventDefault();
      accept(suggestions[Math.max(active, 0)]);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (open && active >= 0) accept(suggestions[active]);
      else submit(draft);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      // Stop the shell's escape (back to list) from also firing.
      e.stopPropagation();
      if (open) setDismissed(true);
      else {
        setDraft(routeQ);
        inputRef.current?.blur();
      }
    }
  };

  return (
    <div className={cn('relative w-full max-w-md', className)}>
      <div
        className={cn(
          'flex h-7 items-center gap-2 rounded-md border bg-background/60 px-2.5 text-sm transition-colors duration-(--duration-fast)',
          'focus-within:border-ring focus-within:bg-background focus-within:ring-2 focus-within:ring-ring/25',
          !focused && 'hover:bg-accent/60',
        )}
      >
        <Search className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
        <input
          ref={inputRef}
          type="search"
          role="combobox"
          aria-label="Search mail"
          aria-expanded={open}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={open && active >= 0 ? `${listId}-${active}` : undefined}
          placeholder="Search mail"
          value={draft}
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => edit(e.target.value)}
          onFocus={() => {
            setFocused(true);
            setDismissed(false);
          }}
          onBlur={() => {
            setFocused(false);
            setActive(-1);
          }}
          onKeyDown={onKeyDown}
          className="h-full min-w-0 flex-1 bg-transparent outline-none placeholder:text-muted-foreground [&::-webkit-search-cancel-button]:hidden"
        />
        {draft && focused ? (
          <button
            type="button"
            aria-label="Clear search"
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => edit('')}
            className="rounded-sm text-muted-foreground hover:text-foreground"
          >
            <X className="size-3.5" />
          </button>
        ) : (
          !focused && <ShortcutKeys keys="/" />
        )}
      </div>

      {open && (
        <div
          id={listId}
          role="listbox"
          aria-label="Search suggestions"
          className="absolute inset-x-0 top-full z-50 mt-1 animate-in overflow-hidden rounded-lg bg-popover text-popover-foreground shadow-md ring-1 ring-foreground/10 duration-(--duration-base) fade-in-0 slide-in-from-top-1"
          onMouseDown={(e) => e.preventDefault()}
        >
          <ul className="max-h-80 overflow-y-auto p-1">
            {suggestions.map((s, i) => (
              <SuggestionRow
                key={s.key}
                id={`${listId}-${i}`}
                suggestion={s}
                active={i === active}
                firstOperator={s.kind === 'operator' && suggestions[i - 1]?.kind === 'recent'}
                onHover={() => setActive(i)}
                onPick={() => accept(s)}
              />
            ))}
          </ul>
          <div className="flex items-center gap-3 border-t border-hairline px-3 py-1.5 text-2xs text-muted-foreground">
            <span className="flex items-center gap-1">
              <Kbd className="font-mono text-2xs">↵</Kbd> search
            </span>
            {op && (
              <span className="flex items-center gap-1">
                <Kbd className="font-mono text-2xs">Tab</Kbd> complete
              </span>
            )}
            <span className="flex items-center gap-1">
              <Kbd className="font-mono text-2xs">Esc</Kbd> close
            </span>
          </div>
        </div>
      )}
    </div>
  );
}

function SuggestionRow({
  id,
  suggestion: s,
  active,
  firstOperator,
  onHover,
  onPick,
}: {
  id: string;
  suggestion: Suggestion;
  active: boolean;
  firstOperator: boolean;
  onHover: () => void;
  onPick: () => void;
}) {
  return (
    <li
      id={id}
      role="option"
      aria-selected={active}
      onMouseMove={onHover}
      onClick={onPick}
      className={cn(
        'flex h-7 cursor-default items-center gap-2 rounded-sm px-2 text-sm',
        active && 'bg-muted',
        firstOperator && 'mt-1.5',
      )}
    >
      {s.kind === 'recent' && <History className="size-3.5 shrink-0 text-muted-foreground" />}
      <span
        className={cn(
          'truncate',
          s.kind === 'operator' && 'font-mono text-xs text-primary',
          s.kind === 'recent' && 'font-mono text-xs',
        )}
      >
        {s.label}
      </span>
      {s.hint && (
        <span className="ml-auto truncate pl-3 text-xs text-muted-foreground">{s.hint}</span>
      )}
    </li>
  );
}
