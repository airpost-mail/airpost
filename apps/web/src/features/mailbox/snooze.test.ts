import { describe, expect, it } from 'vitest';
import { snoozePresets } from './snooze';

const at = (p: ReturnType<typeof snoozePresets>, label: string) =>
  p.find((x) => x.label === label)!.at;
const local = (y: number, mo: number, d: number, h: number, mi = 0) =>
  new Date(y, mo - 1, d, h, mi).getTime();

describe('snoozePresets', () => {
  it('follows the desktop on a weekday afternoon', () => {
    // Wednesday 2026-10-14 14:25.
    const p = snoozePresets(new Date(2026, 9, 14, 14, 25));
    expect(at(p, 'Later today').getTime()).toBe(local(2026, 10, 14, 17));
    expect(at(p, 'Tonight').getTime()).toBe(local(2026, 10, 14, 20));
    expect(at(p, 'Tomorrow').getTime()).toBe(local(2026, 10, 15, 9));
    expect(at(p, 'This weekend').getTime()).toBe(local(2026, 10, 17, 9));
    expect(at(p, 'Next week').getTime()).toBe(local(2026, 10, 19, 9));
    expect(at(p, 'Next month').getTime()).toBe(local(2026, 11, 1, 9));
  });

  it('rolls over late in the evening and at the weekend', () => {
    // Saturday 2026-10-17 21:10.
    const p = snoozePresets(new Date(2026, 9, 17, 21, 10));
    expect(at(p, 'Tonight').getTime()).toBe(local(2026, 10, 18, 0));
    expect(at(p, 'This weekend').getTime()).toBe(local(2026, 10, 24, 9));
    // Sunday: next week is tomorrow.
    const s = snoozePresets(new Date(2026, 9, 18, 10));
    expect(at(s, 'Next week').getTime()).toBe(local(2026, 10, 19, 9));
    expect(at(s, 'This weekend').getTime()).toBe(local(2026, 10, 24, 9));
  });
});
