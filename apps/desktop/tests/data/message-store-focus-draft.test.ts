// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

import { Message } from '../../src/data/models/message';
import { MessageStore } from '../../src/data/stores/message';

describe('MessageStore.focusDraft', () => {
  let saved: { items: Message[]; expanded: any };

  beforeEach(() => {
    saved = { items: MessageStore._items, expanded: MessageStore._itemsExpanded };
    MessageStore._items = [
      new Message({ id: 'm1', threadId: 't1', headerMessageId: 'm1@x', draft: false }),
      new Message({ id: 'd1', threadId: 't1', headerMessageId: 'd1@x', draft: true }),
    ];
    MessageStore._itemsExpanded = {};
  });

  afterEach(() => {
    MessageStore._items = saved.items;
    MessageStore._itemsExpanded = saved.expanded;
    MessageStore._draftToFocus = null;
  });

  it('expands a collapsed draft and hands its id to the message list once', () => {
    spyOn(MessageStore, 'trigger');
    expect(MessageStore.focusDraft('d1@x')).toBe(true);
    expect(MessageStore._itemsExpanded.d1).toBe('explicit');
    expect(MessageStore.trigger).toHaveBeenCalled();
    expect(MessageStore.takeDraftToFocus()).toBe('d1@x');
    expect(MessageStore.takeDraftToFocus()).toBe(null);
  });

  it('keeps an expanded draft as it is', () => {
    MessageStore._itemsExpanded = { d1: 'default' };
    MessageStore.focusDraft('d1@x');
    expect(MessageStore._itemsExpanded.d1).toBe('default');
  });

  it('does nothing for a draft the open thread does not have, or a sent message', () => {
    expect(MessageStore.focusDraft('other@x')).toBe(false);
    expect(MessageStore.focusDraft('m1@x')).toBe(false);
    expect(MessageStore.takeDraftToFocus()).toBe(null);
  });
});
