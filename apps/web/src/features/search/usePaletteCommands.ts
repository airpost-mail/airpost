import { useNavigate, useParams } from '@tanstack/react-router';
import { useTheme } from 'next-themes';
import { categoryDisplayName, isStandardCategory, useWakeSync } from '@/engine';
import { openComposer } from '@/features/composer';
import { setPreferences, usePreferences } from '@/lib/preferences';
import type { PaletteCommand } from './palette';
import { showShortcutsSheet } from './store';
import { useAllCategories } from './useAllCategories';

export type IconKey =
  | 'compose'
  | 'theme'
  | 'density'
  | 'settings'
  | 'shortcuts'
  | 'sync'
  | 'account'
  | 'inbox'
  | 'category';

export type PaletteEntry = PaletteCommand & { icon: IconKey; color?: string };

/** Everything the palette can do besides searching mail. */
export function usePaletteCommands(): PaletteEntry[] {
  const navigate = useNavigate();
  const { accountId: routeAccountId } = useParams({ strict: false });
  const { resolvedTheme, setTheme } = useTheme();
  const { density } = usePreferences();
  const wake = useWakeSync();
  const allCategories = useAllCategories();
  const multiple = allCategories.length > 1;

  const dark = resolvedTheme === 'dark';
  const actions: PaletteEntry[] = [
    {
      id: 'compose',
      group: 'Actions',
      icon: 'compose',
      label: 'Compose',
      keywords: ['new', 'write', 'message', 'email'],
      shortcut: 'c',
      run: () => openComposer({ mode: 'new', accountId: routeAccountId }),
    },
    {
      id: 'theme',
      group: 'Actions',
      icon: 'theme',
      label: dark ? 'Switch to light theme' : 'Switch to dark theme',
      keywords: ['theme', 'appearance', 'mode', 'night', 'day'],
      run: () => setTheme(dark ? 'light' : 'dark'),
    },
    {
      id: 'density',
      group: 'Actions',
      icon: 'density',
      label: density === 'compact' ? 'Use comfortable rows' : 'Use compact rows',
      keywords: ['density', 'spacing', 'layout'],
      run: () => setPreferences({ density: density === 'compact' ? 'comfortable' : 'compact' }),
    },
    {
      id: 'sync',
      group: 'Actions',
      icon: 'sync',
      label: 'Check for new mail',
      keywords: ['sync', 'refresh', 'fetch', 'wake'],
      run: () => wake.mutate(undefined),
    },
    {
      id: 'shortcuts',
      group: 'Actions',
      icon: 'shortcuts',
      label: 'Keyboard shortcuts',
      keywords: ['keys', 'help', 'hotkeys'],
      shortcut: '?',
      run: showShortcutsSheet,
    },
    {
      id: 'settings',
      group: 'Go to',
      icon: 'settings',
      label: 'Settings',
      keywords: ['preferences', 'options', 'accounts'],
      run: () => void navigate({ to: '/settings' }),
    },
  ];

  const places: PaletteEntry[] = allCategories.flatMap(({ account, categories }) => {
    const accountName = account.label || account.emailAddress;
    const inbox = categories.find((c) => c.role === 'inbox') ?? categories[0];
    const go = (folderId: string) => () =>
      void navigate({
        to: '/$accountId/$folderId',
        params: { accountId: account.id, folderId },
      });
    const accountEntry: PaletteEntry[] =
      multiple && inbox
        ? [
            {
              id: `account:${account.id}`,
              group: 'Go to',
              icon: 'account',
              color: account.color,
              label: accountName,
              detail: account.emailAddress !== accountName ? account.emailAddress : undefined,
              keywords: [account.emailAddress, account.provider, 'account'],
              run: go(inbox.id),
            },
          ]
        : [];
    return [
      ...accountEntry,
      ...categories.map<PaletteEntry>((c) => ({
        id: `category:${c.id}`,
        group: 'Go to',
        icon: c.role === 'inbox' ? 'inbox' : 'category',
        label: categoryDisplayName(c),
        detail: multiple ? accountName : undefined,
        keywords: [
          accountName,
          account.emailAddress,
          c.role,
          c.__cls === 'Label' ? 'label' : 'folder',
        ],
        // Idle, list standard mailboxes (or just the accounts when there are several).
        onlyWhenSearching: multiple || !isStandardCategory(c),
        run: go(c.id),
      })),
    ];
  });

  return [...actions, ...places];
}
