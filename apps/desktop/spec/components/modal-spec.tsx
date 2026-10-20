// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import ModalStore from '../../src/flux/stores/modal-store';
import { getPortalContainer } from '../../src/components/kit/portal';
import { wait, pressRealKey } from './kit/kit-spec-helpers';
import { trackCommands } from './kit/layer-spec-helpers';

const dialog = () =>
  getPortalContainer().querySelector('[data-slot="dialog-content"]') as HTMLElement;

const open = (component: React.ReactElement) =>
  ModalStore.openModal({ component, width: 300, height: 200 });

describe('Modal on the UI kit', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
  });

  afterEach(() => {
    ModalStore.closeModal();
  });

  it('opens the kit dialog at the size asked for, with the legacy classes and a hidden title', () => {
    open(<div className="body">Body</div>);
    expect(ModalStore.isModalOpen()).toBe(true);
    const el = dialog();
    expect(el.getAttribute('role')).toBe('dialog');
    expect(el.classList.contains('modal')).toBe(true);
    expect([el.style.width, el.style.height]).toEqual(['300px', '200px']);
    expect(el.querySelector('.body').textContent).toBe('Body');
    expect(el.querySelector('.modal-close')).not.toBe(null);
    const overlay = getPortalContainer().querySelector('[data-slot="dialog-overlay"]');
    expect(overlay.classList.contains('modal-container')).toBe(true);
    const title = document.getElementById(el.getAttribute('aria-labelledby'));
    expect(title.textContent).toBe('Dialog');
  });

  it('focuses the first input, as before the kit', async () => {
    open(
      <div>
        <button>Plain</button>
        <input className="field" />
      </div>
    );
    await wait(20);
    expect(document.activeElement).toBe(dialog().querySelector('.field'));
  });

  it('closes from its close button', () => {
    open(<div />);
    dialog().querySelector<HTMLElement>('.modal-close').click();
    expect(ModalStore.isModalOpen()).toBe(false);
  });

  describe('Escape', function () {
    const commands = trackCommands();

    it('closes the modal without popping the sheet', async () => {
      open(<input />);
      await wait(20);
      await pressRealKey('Escape');
      expect(ModalStore.isModalOpen()).toBe(false);
      expect(commands.count('core:pop-sheet')).toBe(0);
    });
  });
});
