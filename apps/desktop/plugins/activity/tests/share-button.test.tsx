import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import ShareButton from '../lib/dashboard/share-button';
import { getPortalContainer } from '../../../src/components/kit/portal';

describe('Activity ShareButton', () => {
  afterEach(() => cleanup());

  it('is disabled, with the capability store reason as its tooltip', () => {
    const { container } = render(<ShareButton />);
    const button = container.querySelector('[data-slot="button"]') as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.textContent).toBe('Share this Report');

    // The disabled button takes no pointer events, so its focusable wrapper holds the tooltip.
    const trigger = container.querySelector('.share-button') as HTMLElement;
    expect(trigger.contains(button)).toBe(true);
    fireEvent.focus(trigger);
    const tooltip = getPortalContainer().querySelector('[data-slot="tooltip-content"]');
    expect(tooltip.textContent).toContain('Sharing reports: Needs a Tern server');
  });
});
