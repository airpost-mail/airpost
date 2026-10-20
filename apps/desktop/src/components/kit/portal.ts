// SPDX-License-Identifier: GPL-3.0-only
import { isRTL } from '../../intl/localize';

export const PORTAL_ROOT_ID = 'ui-portal-root';

/**
 * The one element per window that kit layers (Dialog, Popover, DropdownMenu, Select, Tooltip)
 * portal into, created on first use as the last child of <body>. It carries `dir` because
 * Popover and Dialog content have none of their own and take CSS direction from where they are
 * portalled (docs/desktop/ui-kit.md 4.1).
 */
export function getPortalContainer(doc: Document = document): HTMLElement {
  let el = doc.getElementById(PORTAL_ROOT_ID);
  if (!el) {
    el = doc.createElement('div');
    el.id = PORTAL_ROOT_ID;
    el.setAttribute('data-ui-portal-root', '');
    if (isRTL) el.setAttribute('dir', 'rtl');
    doc.body.appendChild(el);
  }
  return el;
}
