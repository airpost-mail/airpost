// SPDX-License-Identifier: GPL-3.0-only
// The undo toast's actions on the UI kit.
import React from 'react';
import { render, cleanup } from '@testing-library/react';
import UndoRedoStore from '../../src/data/stores/undo-redo';
import UndoRedoToast from '../../src/features/undo-redo/toast';

describe('UndoRedoToast', () => {
  afterEach(() => cleanup());

  it('undoes from a kit button', () => {
    const block = { id: 'b1', description: 'Archived 1 thread', tasks: [] };
    spyOn(UndoRedoStore, 'getMostRecent').andReturn(block);
    const { container } = render(<UndoRedoToast />);
    UndoRedoStore.trigger();
    const action = container.querySelector('.action') as HTMLButtonElement;
    expect(action.getAttribute('data-slot')).toBe('button');
    expect(action.getAttribute('data-variant')).toBe('ghost');
    expect(action.textContent).toBe('Undo');
    spyOn(AppEnv.commands, 'dispatch');
    action.click();
    expect(AppEnv.commands.dispatch).toHaveBeenCalledWith('core:undo');
  });
});
