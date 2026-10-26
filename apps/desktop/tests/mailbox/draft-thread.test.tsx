import React from 'react';
import ReactDOM from 'react-dom';
import * as Actions from '../../src/data/actions';
import DatabaseStore from '../../src/data/database/store';
import QuerySubscriptionPool from '../../src/data/query/subscription-pool';
import { Thread } from '../../src/data/models/thread';
import { Message } from '../../src/data/models/message';
import DraftThread from '../../src/features/draft-list/draft-thread';

// Rows read without the body attribute have it undefined.
const message = (id: string, minutes: number, extra: Partial<Message> = {}) => {
  const m = new Message({
    id,
    accountId: 'a1',
    threadId: 't1',
    headerMessageId: `${id}@example.com`,
    date: new Date(2026, 9, 26, 10, minutes),
    snippet: `snippet ${id}`,
    ...extra,
  } as any);
  m.body = undefined;
  return m;
};

describe('DraftThread', function () {
  let container: HTMLDivElement;
  let onMessages: (result: Message[]) => void;
  let onThread: (result: Thread | null) => void;
  let component: DraftThread;
  let bodyReads: string[][];
  let bodies: { [id: string]: string | null };
  let onShown: jasmine.Spy;
  let fetchBodies: jasmine.Spy;

  beforeEach(() => {
    container = document.createElement('div');
    bodyReads = [];
    bodies = {};
    onShown = jasmine.createSpy('onShown');
    spyOn(QuerySubscriptionPool, 'addPrivateSubscription').andCallFake((key, sub, cb) => {
      if (key.endsWith('-messages')) onMessages = cb;
      else onThread = cb;
      return () => {};
    });
    const findAll = DatabaseStore.findAll.bind(DatabaseStore);
    spyOn(DatabaseStore, 'findAll').andCallFake((klass: any, where?: any) => {
      if (!where) return findAll(klass);
      return {
        include: () => {
          bodyReads.push(where.id);
          return Promise.resolve(
            where.id.map((id: string) => {
              const m = message(id, 0);
              m.body = bodies[id];
              return m;
            })
          );
        },
      };
    });
    fetchBodies = spyOn(Actions, 'fetchBodies');
    spyOn(Actions, 'fetchFile');
    component = ReactDOM.render(
      <DraftThread threadId="t1" onShown={onShown} />,
      container
    ) as unknown as DraftThread;
  });

  afterEach(() => {
    ReactDOM.unmountComponentAtNode(container);
  });

  it('shows the thread without its drafts, oldest first, once the thread arrives', () => {
    onMessages([message('m2', 5), message('d1', 9, { draft: true }), message('m1', 1)]);
    expect(container.querySelector('.draft-preview-thread')).toBe(null);
    onThread(new Thread({ id: 't1', accountId: 'a1' } as any));
    const items = Array.from(container.querySelectorAll('.message-item-wrap'));
    expect(items.map((el) => el.querySelector('.collapsed-snippet').textContent)).toEqual([
      'snippet m1',
      'snippet m2',
    ]);
    expect(onShown.callCount).toBe(1);
  });

  it('expands the last message and reads only its body', () => {
    bodies.m2 = '<p>hi</p>';
    onMessages([message('m1', 1), message('m2', 5)]);
    expect(component.state.expanded).toEqual({ m2: true });
    expect(bodyReads).toEqual([['m2']]);
    waitsFor(() => component.state.messages[1].body === '<p>hi</p>');
    runs(() => {
      expect(component.state.messages[0].body).toBe(undefined);
    });
  });

  it('keeps read bodies across updates and asks the engine for missing ones once', () => {
    bodies.m1 = null;
    onMessages([message('m1', 1)]);
    waitsFor(() => fetchBodies.callCount === 1);
    runs(() => {
      onMessages([message('m1', 1)]);
    });
    waitsFor(() => bodyReads.length === 2);
    runs(() => {
      bodies.m1 = '<p>arrived</p>';
      onMessages([message('m1', 1)]);
    });
    waitsFor(() => component.state.messages[0].body === '<p>arrived</p>');
    runs(() => {
      onMessages([message('m1', 1, { unread: true })]);
      expect(component.state.messages[0].body).toBe('<p>arrived</p>');
      expect(fetchBodies.callCount).toBe(1);
    });
  });
});
