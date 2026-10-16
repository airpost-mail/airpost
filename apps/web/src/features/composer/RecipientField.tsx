import { X } from 'lucide-react';
import { useId, useImperativeHandle, useRef, useState, type ReactNode, type Ref } from 'react';
import { contactDisplayName, useContacts, type ContactRef } from '@/engine';
import { cn } from '@/lib/utils';
import {
  dedupeContacts,
  emailKey,
  endsWithSeparator,
  formatContact,
  parseRecipients,
  type ParsedRecipients,
} from './recipients';
import type { RecipientFieldHandle } from './useComposerSession';

export interface RecipientFieldProps {
  label: string;
  value: ContactRef[];
  onChange: (next: ContactRef[]) => void;
  /** Addresses already used in the other fields; left out of suggestions. */
  exclude?: ContactRef[];
  trailing?: ReactNode;
  ref?: Ref<RecipientFieldHandle>;
}

export function RecipientField({
  label,
  value,
  onChange,
  exclude = [],
  trailing,
  ref,
}: RecipientFieldProps) {
  const id = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const [text, setText] = useState('');
  const [focused, setFocused] = useState(false);
  const [active, setActive] = useState(0);
  const [dismissed, setDismissed] = useState(false);

  // Every account's contacts, like the desktop's ContactStore: a work contact is a valid
  // recipient from a personal address too.
  const contacts = useContacts(text, { limit: 8 });
  const taken = new Set([...value, ...exclude].map((c) => emailKey(c.email)));
  const suggestions = (contacts.data ?? [])
    .filter((c) => !taken.has(emailKey(c.email)) && !c.h)
    .slice(0, 6);
  const open = focused && !dismissed && text.trim().length > 0 && suggestions.length > 0;
  const activeIndex = Math.min(active, Math.max(0, suggestions.length - 1));

  const add = (list: ContactRef[]) => {
    if (list.length) onChange(dedupeContacts([...value, ...list]));
  };

  const commit = (input = text): ParsedRecipients => {
    const parsed = parseRecipients(input);
    add(parsed.contacts);
    setText(parsed.invalid.join(', '));
    return parsed;
  };

  useImperativeHandle(ref, () => ({
    commit: () => commit(),
    focus: () => inputRef.current?.focus(),
  }));

  const pick = (c: ContactRef) => {
    add([{ email: c.email, name: c.name || undefined }]);
    setText('');
    setActive(0);
  };

  const remove = (c: ContactRef) => onChange(value.filter((x) => x !== c));

  const edit = (c: ContactRef) => {
    remove(c);
    setText(formatContact(c));
    inputRef.current?.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.metaKey || e.ctrlKey) return;
    switch (e.key) {
      case 'ArrowDown':
      case 'ArrowUp':
        if (!open) return;
        e.preventDefault();
        setActive(
          (activeIndex + (e.key === 'ArrowDown' ? 1 : -1) + suggestions.length) %
            suggestions.length,
        );
        return;
      case 'Enter':
        if (open) {
          e.preventDefault();
          pick(suggestions[activeIndex]);
        } else if (text.trim()) {
          e.preventDefault();
          commit();
        }
        return;
      case 'Tab':
        if (open && !e.shiftKey) {
          e.preventDefault();
          pick(suggestions[activeIndex]);
        } else if (text.trim()) {
          commit();
        }
        return;
      case 'Escape':
        if (open) {
          // Keep the composer's escape (minimise) from firing as well.
          e.preventDefault();
          e.stopPropagation();
          setDismissed(true);
        }
        return;
      case 'Backspace':
        if (text === '' && value.length) {
          e.preventDefault();
          remove(value[value.length - 1]);
        }
        return;
    }
  };

  const onPaste = (e: React.ClipboardEvent<HTMLInputElement>) => {
    const pasted = e.clipboardData.getData('text');
    if (!/[,;\n]/.test(pasted)) return;
    e.preventDefault();
    commit(text + pasted);
  };

  const listId = `${id}-suggestions`;

  return (
    <div className="relative flex min-h-9 items-start gap-2 border-b border-hairline px-(--pane-gutter)">
      <label htmlFor={id} className="w-9 shrink-0 pt-2 text-xs text-muted-foreground">
        {label}
      </label>
      <div
        className="flex min-w-0 flex-1 cursor-text flex-wrap items-center gap-1 py-1.5"
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) {
            e.preventDefault();
            inputRef.current?.focus();
          }
        }}
      >
        {value.map((c) => (
          <span
            key={c.email}
            title={formatContact(c)}
            onDoubleClick={() => edit(c)}
            className="group/chip inline-flex h-6 max-w-full items-center gap-0.5 rounded-sm bg-secondary pr-0.5 pl-1.5 text-sm text-secondary-foreground"
          >
            <span className="truncate">{contactDisplayName(c)}</span>
            <button
              type="button"
              aria-label={`Remove ${contactDisplayName(c)}`}
              onClick={() => remove(c)}
              className="grid size-4 place-items-center rounded-sm text-muted-foreground opacity-60 transition-opacity duration-(--duration-fast) group-hover/chip:opacity-100 hover:bg-foreground/10 hover:text-foreground focus-visible:opacity-100"
            >
              <X className="size-3" aria-hidden />
            </button>
          </span>
        ))}
        <input
          ref={inputRef}
          id={id}
          role="combobox"
          aria-expanded={open}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={open ? `${listId}-${activeIndex}` : undefined}
          aria-invalid={!focused && text.trim() !== '' ? true : undefined}
          autoComplete="off"
          spellCheck={false}
          value={text}
          onChange={(e) => {
            const next = e.target.value;
            setDismissed(false);
            setActive(0);
            if (endsWithSeparator(next)) commit(next);
            else setText(next);
          }}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
          onFocus={() => setFocused(true)}
          onBlur={() => {
            setFocused(false);
            if (text.trim()) commit();
          }}
          className="h-6 min-w-24 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground aria-invalid:text-destructive"
        />
      </div>
      {trailing && <div className="flex shrink-0 items-center gap-0.5 pt-1.5">{trailing}</div>}
      {open && (
        <ul
          id={listId}
          role="listbox"
          aria-label={`${label} suggestions`}
          className="absolute top-full left-[calc(var(--pane-gutter)+2.75rem)] z-50 mt-px w-80 max-w-[calc(100%-4rem)] animate-in overflow-hidden rounded-md border bg-popover py-1 text-popover-foreground shadow-md duration-(--duration-fast) fade-in-0"
        >
          {suggestions.map((c, i) => (
            <li
              key={c.id}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === activeIndex}
              onMouseDown={(e) => e.preventDefault()}
              onMouseEnter={() => setActive(i)}
              onClick={() => pick(c)}
              className={cn(
                'flex cursor-default flex-col px-3 py-1.5',
                i === activeIndex && 'bg-accent text-accent-foreground',
              )}
            >
              <span className="truncate text-sm">{c.name || c.email}</span>
              {c.name && <span className="truncate text-xs text-muted-foreground">{c.email}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
