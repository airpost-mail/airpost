// What the agenda shows of an event, read from its stored ICS (the engine indexes the same
// fields for search but keeps them out of the Event JSON). Read-only: building and editing
// ICS moves to the engine with DCP stage 18.
import type { EventJSON } from '@/engine';

export interface EventSummary {
  title: string;
  location: string;
  description: string;
  allDay: boolean;
  /** The master of a recurring event; the agenda shows its stored range, not occurrences. */
  recurring: boolean;
  /** Unix seconds: the stored recurrence range for a master, the event otherwise. */
  start: number;
  end: number;
}

/** Content lines of the first VEVENT, unfolded (RFC 5545 3.1). */
function veventLines(ics: string): string[] {
  const lines = ics.replace(/\r?\n[ \t]/g, '').split(/\r?\n/);
  const start = lines.findIndex((l) => l.toUpperCase() === 'BEGIN:VEVENT');
  if (start < 0) return [];
  const end = lines.findIndex((l, i) => i > start && l.toUpperCase() === 'END:VEVENT');
  return lines.slice(start + 1, end < 0 ? undefined : end);
}

function unescapeText(v: string): string {
  return v.replace(/\\([nN\\,;])/g, (_, c: string) => (c === 'n' || c === 'N' ? '\n' : c));
}

export function summarizeEvent(e: EventJSON): EventSummary {
  const props = new Map<string, { params: string; value: string }>();
  for (const line of veventLines(e.ics)) {
    const colon = line.indexOf(':');
    if (colon < 0) continue;
    const [name, ...params] = line.slice(0, colon).split(';');
    const key = name.toUpperCase();
    if (!props.has(key)) props.set(key, { params: params.join(';'), value: line.slice(colon + 1) });
  }
  const text = (k: string) => unescapeText(props.get(k)?.value ?? '').trim();
  const dtstart = props.get('DTSTART');
  const allDay =
    !!dtstart && (/VALUE=DATE(?!-)/i.test(dtstart.params) || /^\d{8}$/.test(dtstart.value));
  return {
    title: text('SUMMARY') || '(no title)',
    location: text('LOCATION'),
    description: text('DESCRIPTION'),
    allDay,
    recurring: props.has('RRULE') && !e.rid,
    start: e.rs,
    end: e.re,
  };
}
