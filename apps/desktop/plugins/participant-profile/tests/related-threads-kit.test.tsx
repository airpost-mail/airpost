// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, fireEvent, cleanup, act } from '@testing-library/react';
import { Thread } from 'mailspring-exports';
import RelatedThreads from '../lib/sidebar-related-threads';

const threads = [1, 2, 3, 4, 5].map(
  (n) =>
    new Thread({
      id: `t${n}`,
      accountId: TEST_ACCOUNT_ID,
      subject: `Subject ${n}`,
      lastMessageReceivedTimestamp: new Date(2026, 0, n),
    })
);

describe('RelatedThreads toggle on the UI kit', function () {
  afterEach(() => cleanup());

  it('is a link Button that shows the rest of the threads', () => {
    const ref = React.createRef<RelatedThreads>();
    // No contact, so it doesn't query; the threads are set directly.
    const { container } = render(<RelatedThreads ref={ref} contact={undefined} />);
    act(() => ref.current!.setState({ threads }));

    expect(container.querySelectorAll('.related-thread').length).toBe(3);
    const toggle = container.querySelector('button.toggle')!;
    expect(toggle.getAttribute('data-slot')).toBe('button');
    expect(toggle.getAttribute('data-variant')).toBe('link');
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(toggle.textContent).toBe('Show more');

    fireEvent.click(toggle);
    expect(container.querySelectorAll('.related-thread').length).toBe(5);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(toggle.textContent).toBe('Collapse');
  });
});
