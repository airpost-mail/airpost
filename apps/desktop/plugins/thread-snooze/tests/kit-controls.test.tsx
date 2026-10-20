// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, cleanup, fireEvent } from '@testing-library/react';
import { Actions, FocusedPerspectiveStore, Thread } from 'mailspring-exports';
import SnoozePopover from '../lib/popover';
import { QuickActionSnooze, ToolbarSnooze } from '../lib/buttons';
import * as SnoozeActions from '../lib/actions';

const thread = new Thread({ id: 'thread-1', accountId: TEST_ACCOUNT_ID });

describe('thread-snooze kit controls', function () {
  beforeEach(() => {
    spyOn(FocusedPerspectiveStore, 'current').andReturn({ isInbox: () => true } as any);
    spyOn(Actions, 'openPopover');
  });

  afterEach(() => cleanup());

  it('renders the toolbar button as a kit icon button that opens the popover', () => {
    const { container } = render(<ToolbarSnooze items={[thread]} />);
    const button = container.querySelector('.snooze-button');
    expect(button.getAttribute('data-slot')).toBe('icon-button');
    expect(button.classList.contains('btn-toolbar')).toBe(true);
    expect(button.getAttribute('aria-label')).toBe('Snooze');
    fireEvent.click(button);
    expect(Actions.openPopover).toHaveBeenCalled();
  });

  it('renders the quick action as a kit button with the action-snooze class', () => {
    const { container } = render(<QuickActionSnooze thread={thread} />);
    const button = container.querySelector('.action.action-snooze');
    expect(button.getAttribute('data-slot')).toBe('button');
    expect(button.getAttribute('aria-label')).toBe('Snooze');
    fireEvent.click(button);
    expect(Actions.openPopover).toHaveBeenCalled();
  });

  it('renders the snooze times as kit buttons that snooze the threads', () => {
    spyOn(SnoozeActions, 'snoozeThreads');
    spyOn(Actions, 'closePopover');
    spyOn(Actions, 'popSheet');
    const { container } = render(<SnoozePopover threads={[thread]} />);
    const items = container.querySelectorAll('.snooze-item');
    expect(items.length).toBe(6);
    items.forEach((item) => expect(item.getAttribute('data-slot')).toBe('button'));
    fireEvent.click(items[2]);
    expect(SnoozeActions.snoozeThreads).toHaveBeenCalled();
    expect((SnoozeActions.snoozeThreads as any).calls[0].args[0]).toEqual([thread]);
  });
});
