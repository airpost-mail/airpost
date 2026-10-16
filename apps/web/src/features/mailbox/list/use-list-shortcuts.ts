import { useShortcut } from '@/lib/shortcuts';

export interface ListCommands {
  next: () => void;
  prev: () => void;
  open: () => void;
  back: () => void;
  toggleSelect: () => void;
  selectAll: () => void;
  selectNone: () => void;
  archive: () => void;
  trash: () => void;
  spam: () => void;
  star: () => void;
  markUnread: () => void;
  markRead: () => void;
  label: () => void;
  move: () => void;
}

/** The thread list's keyboard map (DESIGN.md, Keyboard). */
export function useListShortcuts(c: ListCommands) {
  const nav = { group: 'Navigation' } as const;
  const thread = { group: 'Thread' } as const;
  useShortcut(['j', 'down'], c.next, { ...nav, description: 'Next thread' });
  useShortcut(['k', 'up'], c.prev, { ...nav, description: 'Previous thread' });
  useShortcut(['enter', 'o'], c.open, { ...nav, description: 'Open thread' });
  useShortcut(['escape', 'u'], c.back, { ...nav, description: 'Back to list' });
  useShortcut('x', c.toggleSelect, { ...thread, description: 'Select thread' });
  useShortcut('* a', c.selectAll, { ...thread, description: 'Select all' });
  useShortcut('* n', c.selectNone, { ...thread, description: 'Select none' });
  useShortcut('e', c.archive, { ...thread, description: 'Archive' });
  useShortcut('#', c.trash, { ...thread, description: 'Trash' });
  useShortcut('!', c.spam, { ...thread, description: 'Spam' });
  useShortcut('s', c.star, { ...thread, description: 'Star / unstar' });
  useShortcut('shift+u', c.markUnread, { ...thread, description: 'Mark unread' });
  useShortcut('shift+i', c.markRead, { ...thread, description: 'Mark read' });
  useShortcut('l', c.label, { ...thread, description: 'Label' });
  useShortcut('v', c.move, { ...thread, description: 'Move to' });
}
