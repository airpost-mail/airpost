// SPDX-License-Identifier: GPL-3.0-only
// Helpers for the UI kit specs.

// The spec clock never fires setTimeout() with no delay (docs/desktop/ui-kit.md finding 5), so
// waits go through the real timer.
export const wait = (ms: number) =>
  new Promise<void>((resolve) => window.originalSetTimeout(resolve, ms));

// A real key press through Chromium, so native behaviour (a focused <button> clicking on Space
// and Enter) runs as in the app. Electron key codes: 'Space', 'Enter', 'Left', 'Right', ...
export async function pressRealKey(keyCode: string) {
  const webContents = AppEnv.getCurrentWindow().webContents;
  webContents.sendInputEvent({ type: 'keyDown', keyCode });
  if (keyCode === 'Enter' || keyCode === 'Space') {
    webContents.sendInputEvent({ type: 'char', keyCode: keyCode === 'Enter' ? '\r' : ' ' });
  }
  // Held briefly, as typed: Radix RadioGroup checks the item arrow focus lands on only while
  // the arrow key is still down, and moves focus in a timeout.
  await wait(30);
  webContents.sendInputEvent({ type: 'keyUp', keyCode });
  await wait(30);
}

// Counts the global commands bound to plain keys (keymaps/base.json) while a spec runs.
export function watchPlainKeyCommands() {
  const counts: { [command: string]: number } = {};
  const commands = [
    'core:messages-page-down',
    'core:focus-item',
    'core:next-item',
    'core:previous-item',
    'core:move-left',
    'core:move-right',
  ];
  const handlers = {};
  for (const command of commands) {
    counts[command] = 0;
    handlers[command] = () => (counts[command] += 1);
  }
  const disposable = AppEnv.commands.add(document.body, handlers);
  return {
    counts,
    total: () => Object.values(counts).reduce((a, b) => a + b, 0),
    dispose: () => disposable.dispose(),
  };
}
