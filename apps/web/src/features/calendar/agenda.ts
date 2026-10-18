// The agenda's days: which stored events show on which day of the range.
import { addDays, format, isSameDay } from 'date-fns';
import type { CalendarJSON, EventJSON } from '@/engine';
import type { EventSummary } from './ics';

export const DAYS = 14;
export const DAY = 86400;

export interface AgendaItem {
  event: EventJSON;
  summary: EventSummary;
  calendar: CalendarJSON | undefined;
}

const utcDate = (t: number) => new Date(t * 1000).toISOString().slice(0, 10);

/** The days of `[from, from + DAYS)` that have events, each with what overlaps it. */
export function agendaDays(items: AgendaItem[], from: Date): { day: Date; items: AgendaItem[] }[] {
  const out: { day: Date; items: AgendaItem[] }[] = [];
  for (let i = 0; i < DAYS; i++) {
    const day = addDays(from, i);
    const start = day.getTime() / 1000;
    const end = addDays(day, 1).getTime() / 1000;
    const today = items.filter(({ summary: s }) => {
      if (s.recurring) {
        // A master's stored range spans every occurrence: show it once, on its first day here.
        const first = Math.max(s.start, from.getTime() / 1000);
        return first >= start && first < end;
      }
      if (s.allDay) {
        // All-day dates are floating: stored as UTC midnights, shown on those dates here.
        const key = format(day, 'yyyy-MM-dd');
        return utcDate(s.start) <= key && key < utcDate(Math.max(s.end, s.start + DAY));
      }
      return s.start < end && s.end > start;
    });
    if (today.length > 0) out.push({ day, items: today });
  }
  return out;
}

export function timeLabel(s: EventSummary, day: Date): string {
  if (s.allDay) return 'All day';
  if (s.recurring) return `${format(new Date(s.start * 1000), 'HH:mm')} · repeats`;
  const start = new Date(s.start * 1000);
  const end = new Date(s.end * 1000);
  const from = isSameDay(start, day) ? format(start, 'HH:mm') : '…';
  const to = isSameDay(end, day) ? format(end, 'HH:mm') : '…';
  return `${from} – ${to}`;
}
