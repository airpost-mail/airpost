// `events.expand` for the mock: one occurrence per stored event, read from its ICS. The
// fixtures have no recurring events; the engine expands those (airpost-pim).
import type { RpcMethods } from '../rpc';
import type { EventJSON, EventOccurrenceJSON } from '../types';
import type { MockStore } from './store';

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

const DAY = 86400;
const utcDay = (t: number) => Math.floor(t / DAY);
const localDay = (t: number) => {
  const d = new Date(t * 1000);
  return Math.round(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / (DAY * 1000));
};

export function occurrenceOf(e: EventJSON): EventOccurrenceJSON {
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
  // All-day dates are floating, stored as UTC midnights; DTEND is exclusive.
  const startDate = allDay ? utcDay(e.rs) : localDay(e.rs);
  const endDate = allDay ? utcDay(e.re) - 1 : localDay(Math.max(e.re - 1, e.rs));
  return {
    id: e.id,
    eventId: e.id,
    accountId: e.aid,
    calendarId: e.cid,
    title: text('SUMMARY'),
    location: text('LOCATION'),
    description: text('DESCRIPTION'),
    startDate,
    endDate: Math.max(endDate, startDate),
    isAllDay: allDay,
    ...(allDay ? {} : { start: e.rs, end: e.re }),
    isCancelled: e.status === 'CANCELLED',
    isPending: e.status === 'TENTATIVE',
    isRecurring: props.has('RRULE'),
  };
}

export function expandEvents(
  s: MockStore,
  p: RpcMethods['events.expand']['params'],
): RpcMethods['events.expand']['result'] {
  const occurrences = [...s.events.values()]
    .filter(
      (e) =>
        (!p.calendarIds || p.calendarIds.includes(e.cid)) &&
        (!p.accountIds || p.accountIds.includes(e.aid)) &&
        (!p.eventIds || p.eventIds.includes(e.id)) &&
        e.re > p.start &&
        e.rs < p.end,
    )
    .sort((a, b) => a.rs - b.rs)
    .map(occurrenceOf);
  return { occurrences };
}
