import React from 'react';
import { Thread } from '../../src/data/models/thread';
import { Message } from '../../src/data/models/message';
import { ButtonDropdown } from '../../src/components/button-dropdown';
import MessageControls from '../../src/features/message-list/message-controls';

const rendered = (message: Message) => {
  const thread = new Thread({ id: 'thread-1', accountId: 'acct' });
  const el = new MessageControls({ thread, message }).render();
  return React.Children.toArray(el.props.children) as React.ReactElement[];
};

describe('MessageControls', () => {
  it('offers reply and forward on a message', () => {
    const message = new Message({ id: 'm', draft: false, from: [], to: [], cc: [] });
    expect(rendered(message).some((c) => c.type === ButtonDropdown)).toBe(true);
  });

  it('offers no reply or forward on a draft that renders as a message', () => {
    const message = new Message({ id: 'd', draft: true, from: [], to: [], cc: [] });
    const children = rendered(message);
    expect(children.some((c) => c.type === ButtonDropdown)).toBe(false);
    expect(children.length).toBe(1);
  });
});
