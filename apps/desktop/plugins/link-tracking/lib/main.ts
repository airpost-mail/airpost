import { localized, ComponentRegistry, ExtensionRegistry } from 'mailspring-exports';
import { HasTutorialTip } from 'mailspring-component-kit';

import LinkTrackingButton from './button';
import LinkTrackingMessageExtension from './message-extension';

const LinkTrackingButtonWithTutorialTip = HasTutorialTip(LinkTrackingButton, {
  title: localized('Track links in this email'),
  instructions: localized(
    'When link tracking is turned on, Tern will notify you when recipients click links in this email.'
  ),
});

export function activate() {
  ComponentRegistry.register(LinkTrackingButtonWithTutorialTip, {
    role: 'Composer:ActionButton',
  });

  ExtensionRegistry.MessageView.register(LinkTrackingMessageExtension);
}

export function serialize() {}

export function deactivate() {
  ComponentRegistry.unregister(LinkTrackingButtonWithTutorialTip);
  ExtensionRegistry.MessageView.unregister(LinkTrackingMessageExtension);
}
