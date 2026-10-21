import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import ThreadSharingButton from '../lib/button';
import { getPortalContainer } from '../../../src/components/kit/portal';

describe('ThreadSharingButton', () => {
  afterEach(() => cleanup());

  it('is a disabled kit toolbar button, with the capability store reason as its tooltip', () => {
    const { container } = render(<ThreadSharingButton items={[]} thread={null} />);
    const button = container.querySelector('[data-slot="icon-button"]') as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.getAttribute('data-variant')).toBe('toolbar');
    expect(button.getAttribute('aria-label')).toBe('Sharing threads: Needs a Tern server');

    // The disabled button takes no pointer events, so its wrapper holds the tooltip.
    const trigger = container.querySelector('.thread-sharing-button') as HTMLElement;
    expect(trigger.contains(button)).toBe(true);
    fireEvent.focus(trigger);
    expect(
      getPortalContainer().querySelector('[data-slot="tooltip-content"]')!.textContent
    ).toContain('Sharing threads: Needs a Tern server');
  });
});
