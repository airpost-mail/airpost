import { render, fireEvent, cleanup, act } from '@testing-library/react';
import proxyquire from 'proxyquire';
import React from 'react';
import { App } from '../../src/platform/app-api';

let stubUpdaterState = null;
let stubUpdaterReleaseVersion = null;
let ipcSendArgs = null;

const patched = proxyquire('../../src/features/notifications/items/update', {
  electron: {
    ipcRenderer: {
      send: (...args) => {
        ipcSendArgs = args;
      },
    },
  },
});

const UpdateNotification = patched.default;

// The component asks main for the updater's state after mounting.
async function mount() {
  const result = render(<UpdateNotification />);
  await act(async () => {
    await Promise.resolve();
  });
  return result;
}

describe('UpdateNotification', function describeBlock() {
  afterEach(cleanup);

  beforeEach(() => {
    stubUpdaterState = 'idle';
    stubUpdaterReleaseVersion = undefined;
    ipcSendArgs = null;
    spyOn(App, 'updaterState').andCallFake(() =>
      Promise.resolve({
        state: stubUpdaterState,
        releaseVersion: stubUpdaterReleaseVersion,
        releaseNotes: 'A new version is available!',
      })
    );
  });

  describe('mounting', () => {
    it('should display a notification once main reports one', async () => {
      stubUpdaterState = 'update-available';
      const { container } = await mount();
      expect(container.querySelector('.notification') !== null).toEqual(true);
    });

    it('should not display a notification if no update is avialable', async () => {
      stubUpdaterState = 'no-update-available';
      const { container } = await mount();
      expect(container.querySelector('.notification') !== null).toEqual(false);
    });

    it('should listen for `window:update-available`', async () => {
      spyOn(AppEnv, 'onUpdateAvailable').andCallThrough();
      await mount();
      expect(AppEnv.onUpdateAvailable).toHaveBeenCalled();
    });
  });

  describe('displayNotification', () => {
    it('should include the version if one is provided', async () => {
      stubUpdaterState = 'update-available';
      stubUpdaterReleaseVersion = '0.515.0-123123';
      const { container } = await mount();
      expect(container.querySelector('.title').textContent.indexOf('0.515.0-123123') >= 0).toBe(
        true
      );
    });

    describe('when the action is taken', () => {
      it('should fire the `application:install-update` IPC event', async () => {
        stubUpdaterState = 'update-available';
        const { container } = await mount();
        fireEvent.click(container.querySelector('#action-0'));
        expect(ipcSendArgs).toEqual(['command', 'application:install-update']);
      });

      it('should dismiss the update notification prompt', async () => {
        stubUpdaterState = 'update-available';
        const { container } = await mount();
        expect(container.querySelector('.notification') !== null).toEqual(true);
        fireEvent.click(container.querySelector('#action-1'));
        expect(container.querySelector('.notification') !== null).toEqual(false);
      });
    });
  });
});
