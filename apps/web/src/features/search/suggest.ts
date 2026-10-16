// Autocomplete for the search box. Pure: the box gathers contacts, categories and recent
// searches and passes them in.
import { format } from 'date-fns';
import {
  categoryDisplayName,
  contactDisplayName,
  type CategoryJSON,
  type ContactJSON,
} from '@/engine';
import { DATE_PRESETS, HAS_VALUES, IS_VALUES, OPERATORS } from './operators';
import { activeOperator, parseDateValue, type SearchField } from './query';

export interface Suggestion {
  key: string;
  kind: 'operator' | 'value' | 'recent';
  label: string;
  hint?: string;
  /** The whole input after accepting. */
  value: string;
  /** Run the search right away instead of continuing to type. */
  submit: boolean;
}

export interface SuggestContext {
  contacts: ContactJSON[];
  /** Categories with the names of the accounts that have them. */
  categories: { category: CategoryJSON; accountName: string }[];
  recent: string[];
  multipleAccounts: boolean;
  now?: Date;
}

const LIMIT = 8;

function quote(v: string) {
  return /\s/.test(v) ? `"${v}"` : v;
}

function valueSuggestions(
  field: SearchField,
  typed: string,
  ctx: SuggestContext,
): { value: string; label: string; hint?: string }[] {
  const t = typed.toLowerCase();
  const now = ctx.now ?? new Date();
  switch (field) {
    case 'is':
      return IS_VALUES.filter((v) => v.startsWith(t)).map((v) => ({ value: v, label: v }));
    case 'has':
      return HAS_VALUES.filter((v) => v.startsWith(t)).map((v) => ({ value: v, label: v }));
    case 'before':
    case 'after': {
      const presets: string[] = DATE_PRESETS.filter((p) => p.startsWith(t));
      if (typed && !presets.includes(t) && parseDateValue(typed, now)) presets.unshift(typed);
      return presets.map((p) => ({
        value: p,
        label: p,
        hint: format(parseDateValue(p, now)!, 'EEE d MMM yyyy'),
      }));
    }
    case 'in': {
      const byValue = new Map<string, { value: string; label: string; accounts: string[] }>();
      for (const { category, accountName } of ctx.categories) {
        const label = categoryDisplayName(category);
        if (t && !label.toLowerCase().includes(t)) continue;
        const value = category.role || label;
        const entry = byValue.get(value) ?? { value, label, accounts: [] };
        if (!entry.accounts.includes(accountName)) entry.accounts.push(accountName);
        byValue.set(value, entry);
      }
      return [...byValue.values()].map((e) => ({
        value: e.value,
        label: e.label,
        hint: ctx.multipleAccounts ? e.accounts.join(', ') : undefined,
      }));
    }
    case 'from':
    case 'to':
      return ctx.contacts.map((c) => ({
        value: c.email,
        label: contactDisplayName(c),
        hint: c.name ? c.email : undefined,
      }));
    case 'subject':
      return [];
  }
}

export function suggest(input: string, ctx: SuggestContext): Suggestion[] {
  const op = activeOperator(input);
  if (op) {
    const head = input.slice(0, op.start);
    return valueSuggestions(op.field, op.value, ctx)
      .slice(0, LIMIT)
      .map((s) => ({
        key: `value:${op.field}:${s.value}`,
        kind: 'value',
        label: s.label,
        hint: s.hint,
        value: `${head}${op.field}:${quote(s.value)} `,
        submit: false,
      }));
  }

  if (!input.trim()) {
    return [
      ...ctx.recent.slice(0, 5).map<Suggestion>((r) => ({
        key: `recent:${r}`,
        kind: 'recent',
        label: r,
        value: r,
        submit: true,
      })),
      ...OPERATORS.map<Suggestion>((o) => ({
        key: `op:${o.field}`,
        kind: 'operator',
        label: `${o.field}:`,
        hint: o.hint,
        value: `${o.field}:`,
        submit: false,
      })),
    ];
  }

  // Typing the start of an operator name: `fr` -> `from:`.
  const m = /(?:^|\s)([A-Za-z]+)$/.exec(input);
  if (!m) return [];
  const word = m[1].toLowerCase();
  const head = input.slice(0, input.length - m[1].length);
  return OPERATORS.filter((o) => o.field.startsWith(word) && o.field !== word).map((o) => ({
    key: `op:${o.field}`,
    kind: 'operator',
    label: `${o.field}:`,
    hint: o.hint,
    value: `${head}${o.field}:`,
    submit: false,
  }));
}
