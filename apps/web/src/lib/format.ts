import { format, isSameDay, isSameYear } from 'date-fns';

/** Thread list dates: time today, day and month this year, full date before. */
export function formatListDate(unixSeconds: number, now = new Date()): string {
  const d = new Date(unixSeconds * 1000);
  if (isSameDay(d, now)) return format(d, 'HH:mm');
  if (isSameYear(d, now)) return format(d, 'd MMM');
  return format(d, 'd MMM yyyy');
}

export function formatFullDate(unixSeconds: number): string {
  return format(new Date(unixSeconds * 1000), "EEE d MMM yyyy 'at' HH:mm");
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}
