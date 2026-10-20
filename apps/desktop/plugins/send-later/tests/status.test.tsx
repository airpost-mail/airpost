// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, cleanup, fireEvent } from '@testing-library/react';
import { Actions, Message, TaskQueue } from 'mailspring-exports';
import SendLaterStatus from '../lib/status';
import { PLUGIN_ID } from '../lib/constants';

describe('SendLaterStatus', function () {
  afterEach(() => cleanup());

  it('cancels the scheduled send from a kit icon button', () => {
    spyOn(TaskQueue, 'findTasks').andReturn([]);
    spyOn(Actions, 'queueTask');
    const draft = new Message({
      id: 'draft-1',
      headerMessageId: 'hmid-1',
      accountId: TEST_ACCOUNT_ID,
      draft: true,
    }).directlyAttachMetadata(PLUGIN_ID, {
      expiration: new Date(Date.now() + 3600 * 1000),
    }) as Message;
    const { container } = render(<SendLaterStatus draft={draft} />);
    const button = container.querySelector('[data-slot="icon-button"]');
    expect(button.getAttribute('aria-label')).toBe('Cancel Send Later');
    fireEvent.click(button);
    expect(Actions.queueTask).toHaveBeenCalled();
  });
});
