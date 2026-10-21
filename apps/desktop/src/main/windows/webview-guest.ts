// SPDX-License-Identifier: GPL-3.0-only
import { shell } from 'electron';

/**
 * The window open handler for <webview> guests (src/components/webview.tsx): a guest never
 * gets a window of its own, and the http(s) links it opens in one go to the default browser.
 */
export function openWebviewPopupExternally(
  { url }: { url: string },
  openExternal: (url: string) => Promise<void> = shell.openExternal
): { action: 'deny' } {
  if (/^https?:\/\/.+/i.test(url)) {
    openExternal(url).catch(() => {});
  }
  return { action: 'deny' };
}
