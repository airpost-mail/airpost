// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import { render, cleanup, fireEvent } from '@testing-library/react';
import { Actions, Message, SendDraftTask, TaskQueue } from 'mailspring-exports';
import EngineEvents from '../../../src/engine/client/events';
import { Feature, Method } from '../../../src/engine/protocol';
import SendLaterStatus from '../lib/status';
import { PLUGIN_ID } from '../lib/constants';

const NOW = 1_800_000_000_000;

function draftWith(metadata: Record<string, unknown> | null) {
  const draft = new Message({
    id: 'draft-1',
    headerMessageId: 'hmid-1',
    accountId: TEST_ACCOUNT_ID,
    draft: true,
  });
  return (metadata ? draft.directlyAttachMetadata(PLUGIN_ID, metadata) : draft) as Message;
}

function sendTask(fields: Partial<SendDraftTask> = {}) {
  const task = new SendDraftTask({});
  task.headerMessageId = 'hmid-1';
  task.status = 'remote';
  Object.assign(task, fields);
  return task;
}

async function flush() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

describe('SendLaterStatus', function () {
  let now: number;
  let tasks: SendDraftTask[];

  beforeEach(() => {
    now = NOW;
    tasks = [];
    spyOn(Date, 'now').andCallFake(() => now);
    spyOn(TaskQueue, 'findTasks').andCallFake(() => tasks);
    spyOn(Actions, 'queueTask');
  });

  afterEach(() => cleanup());

  const label = (container: HTMLElement) => container.querySelector('.time')?.textContent;
  const button = (container: HTMLElement) =>
    container.querySelector('[data-slot="icon-button"]') as HTMLElement;

  it('shows a scheduled send', () => {
    const { container } = render(
      <SendLaterStatus draft={draftWith({ expiration: new Date(NOW + 3600 * 1000) })} />
    );
    expect(label(container)).toContain('Scheduled for');
    expect(button(container).getAttribute('aria-label')).toBe('Cancel Send Later');
  });

  it('counts an undo-send window down each second', () => {
    const draft = draftWith({ expiration: new Date(NOW + 5000), isUndoSend: true });
    const { container } = render(<SendLaterStatus draft={draft} />);
    expect(label(container)).toBe('Sending in 5s');
    expect(button(container).getAttribute('aria-label')).toBe('Cancel send');

    now += 1000;
    advanceClock(1000);
    expect(label(container)).toBe('Sending in 4s');

    now += 4000;
    advanceClock(4000);
    expect(label(container)).toBe('Sending…');
    expect(button(container)).toBe(null);
  });

  it('shows Sending… while the send task is active, once the expiration is cleared', () => {
    tasks = [sendTask()];
    const { container } = render(<SendLaterStatus draft={draftWith({ expiration: null })} />);
    expect(label(container)).toBe('Sending…');
    expect(button(container)).toBe(null);
  });

  it('shows nothing once the send task is complete', () => {
    tasks = [sendTask({ status: 'complete' })];
    const { container } = render(<SendLaterStatus draft={draftWith({ expiration: null })} />);
    expect(label(container)).toBe(undefined);
  });

  it('keeps the offline state for a queued send', () => {
    tasks = [sendTask({ offlineSince: NOW / 1000 })];
    const { container } = render(<SendLaterStatus draft={draftWith({ expiration: null })} />);
    expect(label(container)).toBe('Queued, sends when online');
  });

  it('keeps the error state and dismisses it with a metadata save', () => {
    const { container } = render(
      <SendLaterStatus draft={draftWith({ expiration: null, error: 'no route' })} />
    );
    expect(label(container)).toBe('Not sent: no route');
    fireEvent.click(button(container));
    expect(Actions.queueTask).toHaveBeenCalled();
  });

  describe('cancelling', () => {
    let calls: Array<{ method: string; params: any }>;
    let cancelled: boolean;

    beforeEach(() => {
      calls = [];
      cancelled = true;
      spyOn(AppEnv, 'showErrorDialog');
      EngineEvents.connected([Feature.DraftPipeline], (method, params) => {
        calls.push({ method, params });
        return Promise.resolve({ cancelled });
      });
    });

    afterEach(() => EngineEvents.gone());

    it('cancels through draft.cancelSend', async () => {
      const { container } = render(
        <SendLaterStatus draft={draftWith({ expiration: new Date(NOW + 5000) })} />
      );
      fireEvent.click(button(container));
      await flush();
      expect(calls).toEqual([
        {
          method: Method.DraftCancelSend,
          params: { accountId: TEST_ACCOUNT_ID, headerMessageId: 'hmid-1' },
        },
      ]);
      expect(Actions.queueTask).not.toHaveBeenCalled();
      expect(AppEnv.showErrorDialog).not.toHaveBeenCalled();
    });

    it('says so when the engine had already queued the send', async () => {
      cancelled = false;
      const { container } = render(
        <SendLaterStatus draft={draftWith({ expiration: new Date(NOW + 3600 * 1000) })} />
      );
      fireEvent.click(button(container));
      await flush();
      expect(AppEnv.showErrorDialog).toHaveBeenCalledWith('The message was already sent.');
    });
  });
});
