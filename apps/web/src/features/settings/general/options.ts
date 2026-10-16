import type { Choice } from '../components/ChoiceSelect';
import type { ReadingPanePosition } from '@/lib/preferences';

/** The desktop's choices (core.reading.markAsReadDelay); -1 is "manually". */
export const MARK_READ_DELAYS: Choice<number>[] = [
  { value: 0, label: 'Instantly' },
  { value: 500, label: 'After 0.5 seconds' },
  { value: 2000, label: 'After 2 seconds' },
  { value: -1, label: 'Manually' },
];

/** The desktop's choices (core.sending.undoSend). */
export const UNDO_SEND_DELAYS: Choice<number>[] = [
  { value: 0, label: 'Off' },
  { value: 5000, label: '5 seconds' },
  { value: 15000, label: '15 seconds' },
  { value: 30000, label: '30 seconds' },
  { value: 60000, label: '60 seconds' },
];

export const READING_PANES: Choice<ReadingPanePosition>[] = [
  { value: 'right', label: 'Right' },
  { value: 'bottom', label: 'Below' },
  { value: 'off', label: 'Off' },
];
