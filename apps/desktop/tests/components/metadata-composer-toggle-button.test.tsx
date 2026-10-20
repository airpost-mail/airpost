import React from 'react';
import ReactDOM from 'react-dom';
import { findRenderedDOMComponentWithTag, Simulate } from 'react-dom/test-utils';
import { Message } from 'mailspring-exports';

import MetadataComposerToggleButton from '../../src/components/metadata-composer-toggle-button';
import MTestUtils from '../support/test-utils';

const PLUGIN_ID = 'toggle-spec-plugin';

const renderButton = (draft: Message, unavailableReason?: string) => {
  const session = { changes: { addPluginMetadata: jasmine.createSpy('addPluginMetadata') } };
  const component = MTestUtils.renderIntoDocument(
    <MetadataComposerToggleButton
      iconName="icon.png"
      pluginId={PLUGIN_ID}
      pluginName="Spec Plugin"
      metadataEnabledValue={{ enabled: true }}
      errorMessage={() => 'error'}
      draft={draft}
      session={session as any}
      unavailableReason={unavailableReason}
    />
  ) as any;
  const button = findRenderedDOMComponentWithTag(component, 'button') as HTMLButtonElement;
  return { session, button };
};

describe('MetadataComposerToggleButton', function toggleButtonSpec() {
  beforeEach(() => {
    spyOn(AppEnv.config, 'set');
  });

  it('stays off when the user never turned it on', () => {
    spyOn(AppEnv.config, 'get').andReturn(undefined);
    const { session, button } = renderButton(new Message({ draft: true }));
    expect(session.changes.addPluginMetadata).not.toHaveBeenCalled();
    expect(button.disabled).toBe(false);
  });

  it('turns itself on once the user turned it on before', () => {
    spyOn(AppEnv.config, 'get').andReturn(true);
    const { session, button } = renderButton(new Message({ draft: true }));
    expect(session.changes.addPluginMetadata).toHaveBeenCalledWith(PLUGIN_ID, { enabled: true });
    expect(button.disabled).toBe(false);
  });

  it('applies the default when the feature becomes available while open', () => {
    spyOn(AppEnv.config, 'get').andReturn(true);
    const session = { changes: { addPluginMetadata: jasmine.createSpy('addPluginMetadata') } };
    const container = document.createElement('div');
    const render = (reason?: string) =>
      ReactDOM.render(
        <MetadataComposerToggleButton
          iconName="icon.png"
          pluginId={PLUGIN_ID}
          pluginName="Spec Plugin"
          metadataEnabledValue={{ enabled: true }}
          errorMessage={() => 'error'}
          draft={new Message({ draft: true })}
          session={session as any}
          unavailableReason={reason}
        />,
        container
      );
    render('No server');
    expect(session.changes.addPluginMetadata).not.toHaveBeenCalled();
    render(undefined);
    expect(session.changes.addPluginMetadata).toHaveBeenCalledWith(PLUGIN_ID, { enabled: true });
    ReactDOM.unmountComponentAtNode(container);
  });

  describe('with an unavailableReason', () => {
    it('stays off, is disabled and uses the reason as its tooltip', () => {
      spyOn(AppEnv.config, 'get').andReturn(undefined);
      const { session, button } = renderButton(new Message({ draft: true }), 'No server');
      expect(session.changes.addPluginMetadata).not.toHaveBeenCalled();
      expect(button.disabled).toBe(true);
      expect(button.title).toBe('No server');

      Simulate.click(button);
      expect(session.changes.addPluginMetadata).not.toHaveBeenCalled();
      expect(AppEnv.config.set).not.toHaveBeenCalled();
    });

    it('can still be turned off when the draft already has it on', () => {
      spyOn(AppEnv.config, 'get').andReturn(undefined);
      const draft = new Message({ draft: true });
      draft.directlyAttachMetadata(PLUGIN_ID, { enabled: true });
      const { session, button } = renderButton(draft, 'No server');
      expect(button.disabled).toBe(false);
      expect(button.title).toBe('Disable Spec Plugin. No server');

      Simulate.click(button);
      expect(session.changes.addPluginMetadata).toHaveBeenCalledWith(PLUGIN_ID, null);
    });
  });
});
