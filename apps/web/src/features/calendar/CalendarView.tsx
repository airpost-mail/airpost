// A read-only agenda of every account's calendars, two weeks at a time, the occurrences
// expanded by the engine (`events.expand`).
import { addDays, format, startOfDay } from 'date-fns';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useMemo, useState } from 'react';
import { EmptyState } from '@/components/EmptyState';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Skeleton } from '@/components/ui/skeleton';
import { useAccounts, useCalendars, useEvents } from '@/engine';
import { useShortcut } from '@/lib/shortcuts';
import { agendaDays, DAY, DAYS, timeLabel } from './agenda';

export function CalendarView() {
  const [from, setFrom] = useState(() => startOfDay(new Date()));
  const [hidden, setHidden] = useState<ReadonlySet<string>>(new Set());
  const calendars = useCalendars();
  const { data: accounts = [] } = useAccounts();
  const visible = useMemo(
    () => (calendars.data ?? []).filter((c) => !hidden.has(c.id)).map((c) => c.id),
    [calendars.data, hidden],
  );
  const start = from.getTime() / 1000;
  const range = calendars.data ? { start, end: start + DAYS * DAY, calendarIds: visible } : null;
  const events = useEvents(range);

  const move = (days: number) => setFrom((d) => addDays(d, days));
  const nav = { group: 'Calendar' } as const;
  useShortcut('j', () => move(DAYS), { ...nav, description: 'Next two weeks' });
  useShortcut('k', () => move(-DAYS), { ...nav, description: 'Previous two weeks' });
  useShortcut('t', () => setFrom(startOfDay(new Date())), { ...nav, description: 'Today' });

  const days = useMemo(() => {
    const byId = new Map((calendars.data ?? []).map((c) => [c.id, c]));
    const items = (events.data ?? []).map((occurrence) => ({
      occurrence,
      calendar: byId.get(occurrence.calendarId),
    }));
    return agendaDays(items, from);
  }, [events.data, calendars.data, from]);

  const byAccount = accounts
    .map((a) => ({ account: a, calendars: (calendars.data ?? []).filter((c) => c.aid === a.id) }))
    .filter((g) => g.calendars.length > 0);

  return (
    <div className="flex h-full min-h-0">
      <aside aria-label="Calendars" className="w-56 shrink-0 border-r px-3 py-4">
        {calendars.isLoading && <Skeleton className="h-24 w-full" />}
        {byAccount.map(({ account, calendars: list }) => (
          <section key={account.id} className="mb-4">
            <h2 className="mb-1 truncate text-2xs font-medium text-muted-foreground uppercase">
              {account.label || account.emailAddress}
            </h2>
            {list.map((c) => (
              <label key={c.id} className="flex h-7 items-center gap-2 text-sm">
                <Checkbox
                  checked={!hidden.has(c.id)}
                  onCheckedChange={(on) =>
                    setHidden((h) => {
                      const next = new Set(h);
                      if (on) next.delete(c.id);
                      else next.add(c.id);
                      return next;
                    })
                  }
                />
                <span
                  aria-hidden
                  className="size-2 shrink-0 rounded-full"
                  style={{ backgroundColor: c.color ?? 'currentColor' }}
                />
                <span className="min-w-0 truncate">{c.name}</span>
              </label>
            ))}
          </section>
        ))}
      </aside>
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center gap-2 border-b px-4 py-2">
          <h1 className="text-sm font-semibold">
            {format(from, 'd MMM')} – {format(addDays(from, DAYS - 1), 'd MMM yyyy')}
          </h1>
          <div className="ml-auto flex items-center gap-1">
            <Button variant="ghost" size="sm" onClick={() => setFrom(startOfDay(new Date()))}>
              Today
            </Button>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Previous two weeks"
              onClick={() => move(-DAYS)}
            >
              <ChevronLeft />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Next two weeks"
              onClick={() => move(DAYS)}
            >
              <ChevronRight />
            </Button>
          </div>
        </header>
        <ScrollArea className="min-h-0 flex-1">
          {events.isLoading ? (
            <div className="space-y-2 p-4">
              <Skeleton className="h-6 w-1/3" />
              <Skeleton className="h-10 w-full" />
            </div>
          ) : days.length === 0 ? (
            <EmptyState title="Nothing scheduled" description="No events in these two weeks." />
          ) : (
            <ol aria-label="Agenda" className="divide-y">
              {days.map(({ day, items }) => (
                <li key={day.toISOString()} className="flex gap-4 px-4 py-3">
                  <div className="w-20 shrink-0 text-sm">
                    <div className="font-medium">{format(day, 'EEE d')}</div>
                    <div className="text-2xs text-muted-foreground">{format(day, 'MMM')}</div>
                  </div>
                  <ul className="min-w-0 flex-1 space-y-2">
                    {items.map(({ occurrence: o, calendar }) => (
                      <li key={o.id} className="flex items-start gap-3 text-sm">
                        <span
                          aria-hidden
                          className="mt-1.5 size-2 shrink-0 rounded-full"
                          style={{ backgroundColor: calendar?.color ?? 'currentColor' }}
                        />
                        <span className="w-28 shrink-0 font-mono text-2xs text-muted-foreground tabular">
                          {timeLabel(o, day)}
                        </span>
                        <span className="min-w-0">
                          <span className="block truncate font-medium">
                            {o.title || '(no title)'}
                          </span>
                          {o.location && (
                            <span className="block truncate text-muted-foreground">
                              {o.location}
                            </span>
                          )}
                        </span>
                      </li>
                    ))}
                  </ul>
                </li>
              ))}
            </ol>
          )}
        </ScrollArea>
      </div>
    </div>
  );
}
