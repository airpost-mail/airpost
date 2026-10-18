import { describe, expect, it } from 'vitest';
import { UNDO_SEND_DELAYS, undoSendChoices } from './options';

describe('undoSendChoices', () => {
  it('is the fixed list for one of its values', () => {
    expect(undoSendChoices(5000)).toBe(UNDO_SEND_DELAYS);
  });

  it('adds a value set elsewhere, in order', () => {
    const choices = undoSendChoices(10000);
    expect(choices.map((c) => c.value)).toEqual([0, 5000, 10000, 15000, 30000, 60000]);
    expect(choices.find((c) => c.value === 10000)?.label).toBe('10 seconds');
    expect(undoSendChoices(1000).find((c) => c.value === 1000)?.label).toBe('1 second');
    expect(undoSendChoices(2500).find((c) => c.value === 2500)?.label).toBe('2.5 seconds');
  });
});
