// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup } from '@testing-library/react';
import { Actions, Message } from 'mailspring-exports';
import TemplatePicker from '../lib/picker';

describe('TemplatePicker', () => {
  afterEach(() => cleanup());

  it('is a kit toolbar button that opens the template menu above itself', () => {
    spyOn(Actions, 'openPopover');
    const draft = new Message({ draft: true, headerMessageId: 'tpl-draft' });
    const { container } = render(<TemplatePicker headerMessageId="tpl-draft" draft={draft} />);
    const button = container.querySelector('.btn-templates')!;
    expect(button.getAttribute('data-slot')).toBe('button');
    expect(button.getAttribute('data-variant')).toBe('toolbar');
    expect(button.className).toContain('btn btn-toolbar');
    expect(button.getAttribute('aria-label')).toBe('Quick Reply');
    expect(button.hasAttribute('title')).toBe(false);

    fireEvent.click(button);
    expect(Actions.openPopover).toHaveBeenCalled();
    const [, options] = (Actions.openPopover as unknown as jasmine.Spy).calls[0].args;
    expect(options.direction).toBe('up');
    expect(options.originRect).toEqual(button.getBoundingClientRect());
  });

  it('renders nothing for a plain-text draft', () => {
    const draft = new Message({ draft: true, headerMessageId: 'tpl-draft', plaintext: true });
    const { container } = render(<TemplatePicker headerMessageId="tpl-draft" draft={draft} />);
    expect(container.querySelector('button')).toBe(null);
  });
});
