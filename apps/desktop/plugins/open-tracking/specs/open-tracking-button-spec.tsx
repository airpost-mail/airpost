import React from 'react';
import ReactDOM from 'react-dom';
import { act } from 'react-dom/test-utils';
import { Message, CapabilityStore } from 'mailspring-exports';
import OpenTrackingButton from '../lib/button';
import LinkTrackingButton from '../../link-tracking/lib/button';

// The buttons are memo'd on the draft's metadata; the capability store has to re-render them.
describe('Tracking composer buttons and capability changes', () => {
  let available: boolean;
  let container: HTMLDivElement;

  beforeEach(() => {
    available = false;
    spyOn(CapabilityStore, 'get').andCallFake(() =>
      available
        ? { available: true, source: 'server' }
        : { available: false, source: 'server', reason: 'Needs a Tern server' }
    );
    spyOn(AppEnv.config, 'get').andReturn(undefined);
    container = document.createElement('div');
  });

  afterEach(() => {
    ReactDOM.unmountComponentAtNode(container);
  });

  for (const [name, Button] of [
    ['open tracking', OpenTrackingButton],
    ['link tracking', LinkTrackingButton],
  ] as const) {
    it(`enables and disables the ${name} toggle as the capability flips`, () => {
      const session = { changes: { addPluginMetadata: jasmine.createSpy('addPluginMetadata') } };
      const draft = new Message({ draft: true });
      ReactDOM.render(<Button draft={draft} session={session as any} />, container);
      const button = () => container.querySelector('button') as HTMLButtonElement;
      expect(button().disabled).toBe(true);

      available = true;
      act(() => {
        CapabilityStore.trigger();
      });
      expect(button().disabled).toBe(false);

      available = false;
      act(() => {
        CapabilityStore.trigger();
      });
      expect(button().disabled).toBe(true);
      expect(session.changes.addPluginMetadata).not.toHaveBeenCalled();
    });
  }
});
