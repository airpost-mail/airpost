// Send-later presets. Scheduling stores `{expiration}` (unix seconds) in the draft's
// `send-later` metadata, the plugin id the desktop uses.
import { addDays, format, nextMonday, set } from 'date-fns';

export const SEND_LATER_PLUGIN_ID = 'send-later';

export interface SendLaterPreset {
  label: string;
  at: Date;
}

const at = (d: Date, hours: number) => set(d, { hours, minutes: 0, seconds: 0, milliseconds: 0 });

export function sendLaterPresets(now = new Date()): SendLaterPreset[] {
  const out: SendLaterPreset[] = [];
  if (now.getHours() < 17) out.push({ label: 'This evening', at: at(now, 18) });
  out.push({ label: 'Tomorrow morning', at: at(addDays(now, 1), 8) });
  out.push({ label: 'Tomorrow afternoon', at: at(addDays(now, 1), 13) });
  out.push({ label: 'Monday morning', at: at(nextMonday(now), 8) });
  return out;
}

export function formatSendLater(d: Date): string {
  return format(d, "EEE d MMM 'at' HH:mm");
}
