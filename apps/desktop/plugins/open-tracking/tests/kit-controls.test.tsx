// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { Actions, Message } from 'mailspring-exports';
import OpenTrackingIcon from '../lib/icon';
import OpenTrackingMessageStatus from '../lib/message-status';
import { PLUGIN_ID } from '../lib/constants';

const withOpens = (openCount: number) => {
  const message = new Message({ id: `m-${openCount}`, accountId: TEST_ACCOUNT_ID });
  message.directlyAttachMetadata(PLUGIN_ID, { open_count: openCount, open_data: [] });
  return message;
};

describe('Open tracking controls on the UI kit', function () {
  beforeEach(() => {
    jasmine.unspy(window, 'setTimeout');
    spyOn(Actions, 'openPopover');
  });

  afterEach(() => cleanup());

  describe('OpenTrackingIcon', () => {
    it('is a kit button that opens the popover when the message was opened', () => {
      const thread: any = { __messages: [withOpens(2)] };
      const { container } = render(<OpenTrackingIcon thread={thread} />);
      const icon = container.querySelector('.open-tracking-icon') as HTMLElement;
      expect(icon.tagName).toBe('BUTTON');
      expect(icon.getAttribute('data-slot')).toBe('button');
      expect(icon.getAttribute('aria-label')).toBe('2 opens');
      expect(icon.hasAttribute('title')).toBe(false);
      expect(icon.querySelector('img.opened')).not.toBeNull();
      fireEvent.mouseDown(icon);
      expect(Actions.openPopover).toHaveBeenCalled();
    });

    it('is a plain indicator while unopened', () => {
      const thread: any = { __messages: [withOpens(0)] };
      const { container } = render(<OpenTrackingIcon thread={thread} />);
      const icon = container.querySelector('.open-tracking-icon') as HTMLElement;
      expect(icon.tagName).toBe('DIV');
      expect(icon.getAttribute('aria-label')).toBe('This message has not been opened');
      expect(icon.hasAttribute('title')).toBe(false);
      fireEvent.mouseDown(icon);
      expect(Actions.openPopover).not.toHaveBeenCalled();
    });
  });

  describe('OpenTrackingMessageStatus', () => {
    it('is a kit button when opened', () => {
      const { container } = render(<OpenTrackingMessageStatus message={withOpens(3)} />);
      const status = container.querySelector('.open-tracking-message-status') as HTMLElement;
      expect(status.tagName).toBe('BUTTON');
      expect(status.getAttribute('data-slot')).toBe('button');
      expect(status.classList.contains('opened')).toBe(true);
      expect(status.textContent).toContain('3 opens');
      fireEvent.keyDown(status, { key: 'Enter' });
      expect(Actions.openPopover).toHaveBeenCalled();
    });

    it('stays text when unopened', () => {
      const { container } = render(<OpenTrackingMessageStatus message={withOpens(0)} />);
      const status = container.querySelector('.open-tracking-message-status') as HTMLElement;
      expect(status.tagName).toBe('SPAN');
      expect(status.classList.contains('unopened')).toBe(true);
    });
  });
});
