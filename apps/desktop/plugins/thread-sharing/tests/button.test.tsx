import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import ThreadSharingButton from '../lib/button';
import CapabilityStore from '../../../src/data/stores/capability';
import { getPortalContainer } from '../../../src/components/kit/portal';

describe('ThreadSharingButton', () => {
  afterEach(() => cleanup());

  it('is a disabled kit toolbar button, with the capability store reason as its tooltip', () => {
    const { container } = render(<ThreadSharingButton items={[]} thread={null} />);
    const button = container.querySelector('[data-slot="icon-button"]') as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.getAttribute('data-variant')).toBe('toolbar');
    expect(button.getAttribute('aria-label')).toBe('Share this thread');

    // The disabled button takes no pointer events, so its wrapper holds the tooltip.
    const trigger = container.querySelector('.thread-sharing-button') as HTMLElement;
    expect(trigger.contains(button)).toBe(true);
    fireEvent.focus(trigger);
    expect(
      getPortalContainer().querySelector('[data-slot="tooltip-content"]')!.textContent
    ).toContain('Sharing threads: Needs a Tern server');
  });

  it('is labelled and says why it is disabled when the server offers sharing', () => {
    spyOn(CapabilityStore, 'get').andReturn({ available: true, source: 'server' });
    const { container } = render(<ThreadSharingButton items={[]} thread={null} />);
    const button = container.querySelector('[data-slot="icon-button"]') as HTMLButtonElement;
    expect(button.getAttribute('aria-label')).toBe('Share this thread');
    fireEvent.focus(container.querySelector('.thread-sharing-button') as HTMLElement);
    expect(
      getPortalContainer().querySelector('[data-slot="tooltip-content"]')?.textContent
    ).toContain('Sharing threads comes in a later version.');
  });
});
