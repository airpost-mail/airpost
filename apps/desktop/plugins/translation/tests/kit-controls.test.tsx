// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup, act } from '@testing-library/react';
import { Message, CapabilityStore } from 'mailspring-exports';
import { TranslateMessageHeader } from '../lib/message-header';
import { TranslateComposerButton } from '../lib/composer-button';
import { getPortalContainer } from '../../../src/components/kit/portal';

const message = () =>
  new Message({ id: 'translate-msg', accountId: TEST_ACCOUNT_ID, from: [], body: 'Bonjour' });

describe('Translation controls on the UI kit', function () {
  let config: { [key: string]: any };

  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
    config = { 'core.translation.disabled': [], 'core.translation.automatic': [] };
    const get = AppEnv.config.get.bind(AppEnv.config);
    spyOn(AppEnv.config, 'get').andCallFake((key) => (key in config ? config[key] : get(key)));
    spyOn(AppEnv.config, 'set').andCallFake((key, value) => (config[key] = value));
  });

  afterEach(() => cleanup());

  describe('TranslateMessageHeader', () => {
    const renderDetected = () => {
      const ref = React.createRef<TranslateMessageHeader>();
      const result = render(<TranslateMessageHeader ref={ref} message={message()} />);
      // Detection runs in the message frame; the spec sets its result.
      act(() => ref.current!.setState({ detected: 'fr' }));
      return { ...result, header: ref.current! };
    };

    it('renders kit buttons for Translate and Options', () => {
      const { container } = renderDetected();
      const actions = Array.from(container.querySelectorAll('.actions .action')) as HTMLElement[];
      expect(actions.length).toBe(2);
      for (const action of actions) {
        expect(action.tagName).toBe('BUTTON');
        expect(action.getAttribute('data-slot')).toBe('button');
        expect(action.getAttribute('data-variant')).toBe('ghost');
      }
      expect(actions[0].textContent).toBe('Translate');
      expect(actions[1].textContent).toBe('Options');
      expect(actions[1].getAttribute('aria-haspopup')).toBe('menu');
      expect(container.querySelector('.button-dropdown')).toBeNull();
    });

    it('opens the options in a kit dropdown menu', () => {
      const { container } = renderDetected();
      const options = container.querySelector('.action-options')!;
      fireEvent.keyDown(options, { key: 'Enter' });
      const menu = getPortalContainer().querySelector('[data-slot="dropdown-menu-content"]')!;
      expect(menu).not.toBeNull();
      const items = Array.from(menu.querySelectorAll('[data-slot="dropdown-menu-item"]'));
      expect(items.map((i) => i.textContent)).toEqual([
        'Always translate French',
        'Never translate French',
        'Reset translation settings',
      ]);
      expect(menu.querySelectorAll('[data-slot="dropdown-menu-separator"]').length).toBe(1);
    });

    it('turns on automatic translation from the menu', () => {
      const { container, header } = renderDetected();
      const translate = spyOn(header, '_onTranslate');
      fireEvent.keyDown(container.querySelector('.action-options')!, { key: 'Enter' });
      const always = getPortalContainer().querySelector('[data-slot="dropdown-menu-item"]')!;
      fireEvent.click(always);
      expect(config['core.translation.automatic']).toEqual(['fr']);
      expect(translate).toHaveBeenCalledWith('manual');
      expect(getPortalContainer().querySelector('[data-slot="dropdown-menu-content"]')).toBeNull();
    });
  });

  describe('TranslateComposerButton', () => {
    const session: any = { changes: { add: () => {}, commit: () => {} } };

    it('is a kit toolbar button with its label as a tooltip on the wrapper', () => {
      spyOn(CapabilityStore, 'unavailableMessage').andReturn('Set up a translation provider.');
      const draft = new Message({ id: 'd', draft: true, accountId: TEST_ACCOUNT_ID });
      const { container } = render(<TranslateComposerButton draft={draft} session={session} />);
      const button = container.querySelector('.btn-translate') as HTMLButtonElement;
      expect(button.getAttribute('data-slot')).toBe('button');
      expect(button.getAttribute('data-variant')).toBe('toolbar');
      expect(button.classList.contains('btn-toolbar')).toBe(true);
      expect(button.hasAttribute('title')).toBe(false);
      expect(button.parentElement!.classList.contains('btn-translate-wrap')).toBe(true);
      expect(button.parentElement!.getAttribute('data-slot')).toBe('tooltip-trigger');
    });
  });
});
