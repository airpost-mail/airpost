import React from 'react';
import ReactDOM from 'react-dom';
import { findRenderedDOMComponentWithClass } from 'react-dom/test-utils';

import { Message, TestUtils } from 'mailspring-exports';
import OpenTrackingIcon from '../lib/icon';
import { PLUGIN_ID } from '../lib/constants';

const { renderIntoDocument } = TestUtils;

type FakeThread = { __messages: Message[] };

function makeIcon(thread: FakeThread, props = {}) {
  // For a class component, renderIntoDocument returns the instance.
  return renderIntoDocument(<OpenTrackingIcon {...props} thread={thread} />) as OpenTrackingIcon;
}

function find(component: React.Component, className: string) {
  return ReactDOM.findDOMNode(
    findRenderedDOMComponentWithClass(component, className)
  ) as HTMLElement;
}

function addOpenMetadata(obj: Message, openCount: number) {
  obj.directlyAttachMetadata(PLUGIN_ID, { open_count: openCount });
}

describe('Open tracking icon', function openTrackingIcon() {
  let thread: FakeThread;
  let messages: Message[];

  beforeEach(() => {
    thread = { __messages: [] };
  });

  it('shows no icon if the thread has no messages', () => {
    const icon = ReactDOM.findDOMNode(makeIcon(thread)) as HTMLElement;
    expect(icon.children.length).toEqual(0);
  });

  it('shows no icon if the thread messages have no metadata', () => {
    thread.__messages.push(new Message({}));
    thread.__messages.push(new Message({}));
    const icon = ReactDOM.findDOMNode(makeIcon(thread)) as HTMLElement;
    expect(icon.children.length).toEqual(0);
  });

  describe('With messages and metadata', () => {
    beforeEach(() => {
      messages = [new Message({}), new Message({}), new Message({ draft: true })];
      thread.__messages.push(...messages);
    });

    it('shows no icon if metadata is malformed', () => {
      messages[0].directlyAttachMetadata(PLUGIN_ID, { gar: 'bage' });
      const icon = ReactDOM.findDOMNode(makeIcon(thread)) as HTMLElement;
      expect(icon.children.length).toEqual(0);
    });

    it('shows an unopened icon if last non draft message has metadata and is unopened', () => {
      addOpenMetadata(messages[0], 1);
      addOpenMetadata(messages[1], 0);
      const icon = find(makeIcon(thread), 'open-tracking-icon');
      expect(icon.children.length).toEqual(1);
      expect(icon.querySelector('img.unopened')).not.toBeNull();
      expect(icon.querySelector('img.opened')).toBeNull();
    });

    it('shows an opened icon if last non draft message with metadata is opened', () => {
      addOpenMetadata(messages[0], 0);
      addOpenMetadata(messages[1], 1);
      const icon = find(makeIcon(thread), 'open-tracking-icon');
      expect(icon.children.length).toEqual(1);
      expect(icon.querySelector('img.unopened')).toBeNull();
      expect(icon.querySelector('img.opened')).not.toBeNull();
    });
  });
});
