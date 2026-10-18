// Snooze presets, the desktop's (thread-snooze `snooze-popover.tsx`, `DateUtils`): mornings
// are 9:00, evenings 20:00.
import { addDays, addHours, addMonths, format, nextMonday, nextSaturday, set } from 'date-fns';

export interface SnoozePreset {
  label: string;
  at: Date;
}

const MORNING = 9;
const EVENING = 20;

const oclock = (d: Date, hours = d.getHours()) =>
  set(d, { hours, minutes: 0, seconds: 0, milliseconds: 0 });

export function snoozePresets(now = new Date()): SnoozePreset[] {
  return [
    { label: 'Later today', at: oclock(addHours(now, 3)) },
    {
      label: 'Tonight',
      at: now.getHours() >= EVENING ? oclock(addDays(now, 1), 0) : oclock(now, EVENING),
    },
    { label: 'Tomorrow', at: oclock(addDays(now, 1), MORNING) },
    { label: 'This weekend', at: oclock(nextSaturday(now), MORNING) },
    { label: 'Next week', at: oclock(nextMonday(now), MORNING) },
    { label: 'Next month', at: oclock(set(addMonths(now, 1), { date: 1 }), MORNING) },
  ];
}

export function formatSnooze(d: Date): string {
  return format(d, "EEE d MMM 'at' HH:mm");
}
