// SPDX-License-Identifier: GPL-3.0-only
// Helpers for the kit layer specs (Dialog, Popover, DropdownMenu, Select, Tooltip).
import { cleanup } from '@testing-library/react';
import * as intl from '../../src/shared/intl';
import { PORTAL_ROOT_ID } from '../../src/components/kit/portal';

const removePortalContainer = () => document.getElementById(PORTAL_ROOT_ID)?.remove();

/** Runs `fn` with isRTL on and a fresh portal container, which then carries dir="rtl". */
export function withRTLPortal(fn: () => void) {
  const wasRTL = intl.isRTL;
  removePortalContainer();
  (intl as { isRTL: boolean }).isRTL = true;
  try {
    fn();
  } finally {
    cleanup();
    (intl as { isRTL: boolean }).isRTL = wasRTL;
    removePortalContainer();
  }
}

const TRACKED = [
  'core:pop-sheet',
  'core:next-item',
  'core:previous-item',
  'core:focus-item',
  'core:messages-page-down',
  'core:remove-from-view',
  'core:select-all',
];

/** Counts the global commands a test fires. Call inside a describe block. */
export function trackCommands() {
  let counts: { [command: string]: number } = {};
  let disposable: { dispose(): void } = null;
  beforeEach(() => {
    counts = {};
    const handlers = {};
    for (const command of TRACKED) {
      handlers[command] = () => (counts[command] = (counts[command] || 0) + 1);
    }
    disposable = AppEnv.commands.add(document.body, handlers);
  });
  afterEach(() => disposable.dispose());
  return { count: (command: string) => counts[command] || 0 };
}
