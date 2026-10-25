// The agenda's days: which occurrences show on which day of the range.
import { addDays, format, isSameDay } from 'date-fns';
import type { CalendarJSON, EventOccurrenceJSON } from '@/engine';

export const DAYS = 14;
export const DAY = 86400;

export interface AgendaItem {
  occurrence: EventOccurrenceJSON;
  calendar: CalendarJSON | undefined;
}

/** A local day as `events.expand` counts it: days from 1970-01-01. */
export function calendarDay(day: Date): number {
  return Math.round(Date.UTC(day.getFullYear(), day.getMonth(), day.getDate()) / (DAY * 1000));
}

/** The days of `[from, from + DAYS)` that have occurrences, each with those covering it. */
export function agendaDays(items: AgendaItem[], from: Date): { day: Date; items: AgendaItem[] }[] {
  const out: { day: Date; items: AgendaItem[] }[] = [];
  for (let i = 0; i < DAYS; i++) {
    const day = addDays(from, i);
    const n = calendarDay(day);
    const today = items.filter(({ occurrence: o }) => o.startDate <= n && n <= o.endDate);
    if (today.length > 0) out.push({ day, items: today });
  }
  return out;
}

export function timeLabel(o: EventOccurrenceJSON, day: Date): string {
  if (o.isAllDay || o.start === undefined || o.end === undefined) return 'All day';
  const start = new Date(o.start * 1000);
  const end = new Date(o.end * 1000);
  const from = isSameDay(start, day) ? format(start, 'HH:mm') : '…';
  const to = isSameDay(end, day) ? format(end, 'HH:mm') : '…';
  return `${from} – ${to}`;
}
