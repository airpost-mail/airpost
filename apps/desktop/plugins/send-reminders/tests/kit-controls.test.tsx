// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, cleanup, fireEvent } from '@testing-library/react';
import { Actions, Thread } from 'mailspring-exports';
import SendRemindersPopover from '../lib/popover';
import SendRemindersPopoverButton from '../lib/popover-button';
import { ScheduledReminderThreadHeader } from '../lib/headers';
import { PLUGIN_ID } from '../lib/constants';

const expiration = new Date(Date.now() + 3600 * 1000);

function threadWithReminder() {
  return new Thread({ id: 'thread-1', accountId: TEST_ACCOUNT_ID }).directlyAttachMetadata(
    PLUGIN_ID,
    { expiration }
  ) as Thread;
}

describe('send-reminders kit controls', function () {
  afterEach(() => cleanup());

  it('renders the toolbar button as a kit icon button labelled for the reminder state', () => {
    spyOn(Actions, 'openPopover');
    const { container } = render(<SendRemindersPopoverButton thread={threadWithReminder()} />);
    const button = container.querySelector('.send-reminders-toolbar-button');
    expect(button.getAttribute('data-slot')).toBe('icon-button');
    expect(button.classList.contains('btn-toolbar')).toBe(true);
    expect(button.getAttribute('aria-label')).toBe('Edit Reminder');
    fireEvent.click(button);
    expect(Actions.openPopover).toHaveBeenCalled();
  });

  it('clears the reminder from the kit button in the popover footer', () => {
    const onCancelReminder = jasmine.createSpy('onCancelReminder');
    const { container } = render(
      <SendRemindersPopover
        reminderDate={expiration}
        onRemind={() => {}}
        onCancelReminder={onCancelReminder}
      />
    );
    const button = container.querySelector('.btn-cancel');
    expect(button.getAttribute('data-slot')).toBe('button');
    fireEvent.click(button);
    expect(onCancelReminder).toHaveBeenCalled();
  });

  it('cancels a scheduled reminder from the thread header link button', () => {
    spyOn(Actions, 'queueTask');
    const { container } = render(<ScheduledReminderThreadHeader thread={threadWithReminder()} />);
    const button = container.querySelector('.clear-reminder');
    expect(button.tagName).toBe('BUTTON');
    expect(button.getAttribute('data-variant')).toBe('link');
    fireEvent.click(button);
    expect(Actions.queueTask).toHaveBeenCalled();
  });
});
