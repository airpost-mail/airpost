// SPDX-License-Identifier: GPL-3.0-only
import fs from 'fs';
import path from 'path';
import React from 'react';
import { render, cleanup } from '@testing-library/react';
import { useDirection } from '@radix-ui/react-direction';
import * as intl from '../../src/intl/localize';
import ModalStore from '../../src/data/stores/modal';
import PopoverStore from '../../src/data/stores/popover';
import { UIProvider } from '../../src/components/kit/provider';

const appDir = path.resolve(__dirname, '..', '..');

// Every ReactDOM.render root, wrapped in UIProvider (docs/desktop/ui-kit.md 4.2). A new root has
// to wrap its tree and be listed here.
const ROOTS = {
  'src/window/app-env.ts': 1,
  'src/data/stores/modal.tsx': 3,
  'src/data/stores/popover.tsx': 3,
  'src/features/onboarding/pages/create-page-for-form.tsx': 1,
};

const DirProbe = (props: { id: string }) => <span id={props.id}>{useDirection()}</span>;

const sourceFiles = (dir: string): string[] =>
  (fs.readdirSync(dir, { recursive: true }) as string[])
    .filter((f) => /\.(tsx?|jsx?)$/.test(f))
    .filter((f) => !f.split(path.sep).some((part) => ['node_modules', 'tests'].includes(part)))
    .map((f) => path.join(dir, f));

describe('UIProvider', function () {
  let wasRTL: boolean;

  beforeEach(() => {
    wasRTL = intl.isRTL;
  });

  afterEach(() => {
    (intl as { isRTL: boolean }).isRTL = wasRTL;
    cleanup();
  });

  it('gives Radix the UI direction', () => {
    const { container, unmount } = render(
      <UIProvider>
        <DirProbe id="ltr" />
      </UIProvider>
    );
    expect(container.textContent).toBe('ltr');
    unmount();
    (intl as { isRTL: boolean }).isRTL = true;
    const rtl = render(
      <UIProvider>
        <DirProbe id="rtl" />
      </UIProvider>
    );
    expect(rtl.container.textContent).toBe('rtl');
  });

  it('wraps what ModalStore and PopoverStore render', () => {
    (intl as { isRTL: boolean }).isRTL = true;
    let modalDir = null;
    ModalStore.openModal(
      { component: <DirProbe id="modal-probe" />, width: 100, height: 100 },
      () => {
        modalDir = document.getElementById('modal-probe').textContent;
      }
    );
    expect(modalDir).toBe('rtl');
    ModalStore.closeModal();

    const anchor = document.createElement('div');
    document.body.appendChild(anchor);
    let popoverDir = null;
    PopoverStore.openPopover(<DirProbe id="popover-probe" />, {
      originRect: { top: 0, left: 0, width: 10, height: 10, bottom: 10, right: 10 },
      direction: 'down',
      fallbackDirection: undefined,
      closeOnAppBlur: false,
      callback: () => {
        popoverDir = document.getElementById('popover-probe').textContent;
      },
    });
    expect(popoverDir).toBe('rtl');
    PopoverStore.closePopover();
    anchor.remove();
  });

  it('wraps every ReactDOM.render call in the app', () => {
    const files = [
      ...sourceFiles(path.join(appDir, 'src')),
      ...sourceFiles(path.join(appDir, 'plugins')),
    ];
    const found = {};
    for (const file of files) {
      const count = (fs.readFileSync(file, 'utf8').match(/ReactDOM\.render\(/g) || []).length;
      if (count) found[path.relative(appDir, file).split(path.sep).join('/')] = count;
    }
    expect(found).toEqual(ROOTS);
    for (const file of Object.keys(ROOTS)) {
      expect(fs.readFileSync(path.join(appDir, file), 'utf8')).toContain('UIProvider');
    }
  });
});
