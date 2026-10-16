import { format } from 'date-fns';
import { parseDateValue, type SearchField, type SearchToken } from './query';

export interface OperatorInfo {
  field: SearchField;
  label: string;
  hint: string;
}

export const OPERATORS: OperatorInfo[] = [
  { field: 'from', label: 'From', hint: 'Sender name or address' },
  { field: 'to', label: 'To', hint: 'Recipient, including cc' },
  { field: 'subject', label: 'Subject', hint: 'Words in the subject' },
  { field: 'in', label: 'In', hint: 'Folder or label' },
  { field: 'is', label: 'Is', hint: 'unread, read, starred' },
  { field: 'has', label: 'Has', hint: 'attachment' },
  { field: 'after', label: 'After', hint: 'On or after a date: 2026-01-31, 7d, yesterday' },
  { field: 'before', label: 'Before', hint: 'Before a date' },
];

export const IS_VALUES = ['unread', 'read', 'starred', 'unstarred'] as const;
export const HAS_VALUES = ['attachment'] as const;
export const DATE_PRESETS = ['today', 'yesterday', '7d', '30d', '1y'] as const;

const FLAG_LABELS: Record<string, string> = {
  'is:unread': 'Unread',
  'is:read': 'Read',
  'is:starred': 'Starred',
  'is:unstarred': 'Not starred',
  'has:attachment': 'Has attachment',
};

export function operatorLabel(field: SearchField): string {
  return OPERATORS.find((o) => o.field === field)?.label ?? field;
}

/** How a filter reads in a chip: `{label: 'From', value: 'ana'}` or `{value: 'Unread'}`. */
export function describeToken(
  t: SearchToken,
  now = new Date(),
): { label?: string; value: string; invalid?: boolean } {
  const flag = FLAG_LABELS[`${t.field}:${t.value}`];
  if (flag) return { value: flag };
  if (t.field === 'before' || t.field === 'after') {
    const d = parseDateValue(t.value, now);
    return {
      label: operatorLabel(t.field),
      value: d ? format(d, 'd MMM yyyy') : t.value,
      invalid: !d,
    };
  }
  return { label: operatorLabel(t.field), value: t.value };
}
