// SPDX-License-Identifier: GPL-3.0-only
import { getPortalContainer, PORTAL_ROOT_ID } from '../../src/components/kit/portal';
import { withRTLPortal } from '../support/kit-layer-helpers';

describe('kit portal container', function () {
  it('is one element per window, at the end of <body>', () => {
    const el = getPortalContainer();
    expect(el.id).toBe(PORTAL_ROOT_ID);
    expect(el.hasAttribute('data-ui-portal-root')).toBe(true);
    expect(el.parentElement).toBe(document.body);
    expect(getPortalContainer()).toBe(el);
    expect(document.querySelectorAll(`#${PORTAL_ROOT_ID}`).length).toBe(1);
  });

  it('is created again at the end of <body> if it was removed', () => {
    getPortalContainer().remove();
    const el = getPortalContainer();
    expect(document.body.lastElementChild).toBe(el);
  });

  it('carries dir only in RTL', () => {
    expect(getPortalContainer().hasAttribute('dir')).toBe(false);
    withRTLPortal(() => {
      expect(getPortalContainer().getAttribute('dir')).toBe('rtl');
    });
    expect(getPortalContainer().hasAttribute('dir')).toBe(false);
  });

  it('stacks above the legacy modal container (base.css is loaded in spec windows)', () => {
    const style = getComputedStyle(getPortalContainer());
    expect(style.zIndex).toBe('1100');
    expect(style.position).toBe('relative');
    expect(style.isolation).toBe('isolate');
  });
});
