// SPDX-License-Identifier: GPL-3.0-only
// The onboarding pages' controls on the UI kit.
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { Account } from '../../src/data/models/account';
import { AccountStore } from '../../src/data/stores/account';
import * as OnboardingActions from '../../src/features/onboarding/actions';
import PageTopBar from '../../src/features/onboarding/pages/top-bar';
import TutorialPage from '../../src/features/onboarding/pages/tutorial';
import InitialPreferencesPage from '../../src/features/onboarding/pages/initial-preferences';
import { getPortalContainer } from '../../src/components/kit/portal';

describe('Onboarding kit controls', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => cleanup());

  describe('the top bar', () => {
    it('closes the window on the first page', () => {
      spyOn(AccountStore, 'accounts').andReturn([new Account({ id: 'a1' } as any)]);
      spyOn(AppEnv, 'close');
      const { container } = render(<PageTopBar pageDepth={1} />);
      const button = container.querySelector('[data-slot="icon-button"]') as HTMLButtonElement;
      expect(button.className).toContain('close');
      expect(button.getAttribute('aria-label')).toBe('Close');
      button.click();
      expect(AppEnv.close).toHaveBeenCalled();
    });

    it('goes back on later pages, and hides when moving back is not allowed', () => {
      spyOn(OnboardingActions, 'moveToPreviousPage');
      const { container, rerender } = render(<PageTopBar pageDepth={2} allowMoveBack />);
      const button = container.querySelector('[data-slot="icon-button"]') as HTMLButtonElement;
      expect(button.className).toContain('back');
      expect(button.getAttribute('aria-label')).toBe('Back');
      button.click();
      expect(OnboardingActions.moveToPreviousPage).toHaveBeenCalled();

      rerender(<PageTopBar pageDepth={2} />);
      expect(container.querySelector('[data-slot="icon-button"]')).toBe(null);
    });
  });

  it('renders the tutorial footer as kit buttons with their layout classes', () => {
    spyOn(OnboardingActions, 'moveToPage');
    const { container } = render(<TutorialPage />);
    const buttons = Array.from(container.querySelectorAll('.footer [data-slot="button"]'));
    expect(buttons.map((b) => b.textContent)).toEqual(['Back', 'Get Started']);
    expect(buttons[0].className).toContain('btn-prev');
    expect(buttons[1].className).toContain('btn-next');
    expect(buttons[1].getAttribute('data-size')).toBe('lg');
    (buttons[1] as HTMLButtonElement).click();
    expect(OnboardingActions.moveToPage).toHaveBeenCalledWith('account-choose');
  });

  describe('the initial preferences', () => {
    let config: { [key: string]: any };

    beforeEach(() => {
      config = { 'core.workspace.mode': 'split', 'core.keymapTemplate': 'Gmail' };
      const get = AppEnv.config.get.bind(AppEnv.config);
      spyOn(AppEnv.config, 'get').andCallFake((key?: string) =>
        key && key in config ? config[key] : get(key)
      );
      spyOn(AppEnv.config, 'set').andCallFake((key: string, value: any) => {
        config[key] = value;
      });
      spyOn(AccountStore, 'accounts').andReturn([
        new Account({ id: 'a1', provider: 'imap', emailAddress: 'a@example.com' } as any),
      ]);
    });

    it('picks the layout from a radio group of tiles', () => {
      const { container } = render(<InitialPreferencesPage />);
      const group = container.querySelector('[role="radiogroup"]');
      expect(group.getAttribute('aria-label')).toBe('Layout');
      const tiles = Array.from(group.querySelectorAll('[role="radio"]')) as HTMLElement[];
      expect(tiles.map((t) => t.getAttribute('aria-label'))).toEqual([
        'Reading Pane Off',
        'Reading Pane On',
      ]);
      expect(tiles.map((t) => t.getAttribute('aria-checked'))).toEqual(['false', 'true']);
      expect(tiles[1].className).toContain('appearance-mode active');
      fireEvent.click(tiles[0]);
      expect(config['core.workspace.mode']).toBe('list');
    });

    it('picks the shortcut set from a select of the keymap templates', async () => {
      const { container } = render(<InitialPreferencesPage />);
      // The templates are read from disk.
      const trigger = container.querySelector('[data-slot="select-trigger"]');
      for (let i = 0; i < 50 && !trigger.textContent; i++) {
        await new Promise((resolve) => window.originalSetTimeout(resolve, 10));
      }
      expect(trigger.getAttribute('aria-label')).toBe('Shortcuts');
      fireEvent.keyDown(trigger, { key: 'Enter' });
      const items = Array.from(
        getPortalContainer().querySelectorAll(
          '[data-slot="select-content"][data-state="open"] [data-slot="select-item"]'
        )
      );
      const outlook = items.find((i) => i.textContent.startsWith('Outlook'));
      expect(outlook).toBeDefined();
      fireEvent.keyDown(outlook, { key: 'Enter' });
      expect(config['core.keymapTemplate']).toBe(outlook.textContent);
    });
  });
});
