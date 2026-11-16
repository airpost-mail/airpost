import { localized, ComponentRegistry } from 'mailspring-exports';
import { HasTutorialTip } from 'mailspring-component-kit';
import OpenTrackingButton from './button';
import OpenTrackingIcon from './icon';
import OpenTrackingMessageStatus from './message-status';

const OpenTrackingButtonWithTutorialTip = HasTutorialTip(OpenTrackingButton, {
  title: localized('See when recipients open this email'),
  instructions: localized(
    'When enabled, Airpost will notify you as soon as someone reads this message. Sending to a group? Airpost shows you which recipients opened your email so you can follow up with precision.'
  ),
});

export function activate() {
  ComponentRegistry.register(OpenTrackingButtonWithTutorialTip, { role: 'Composer:ActionButton' });

  ComponentRegistry.register(OpenTrackingIcon, { role: 'ThreadListIcon' });

  ComponentRegistry.register(OpenTrackingMessageStatus, { role: 'MessageHeaderStatus' });
}

export function serialize() {}

export function deactivate() {
  ComponentRegistry.unregister(OpenTrackingButtonWithTutorialTip);
  ComponentRegistry.unregister(OpenTrackingIcon);
  ComponentRegistry.unregister(OpenTrackingMessageStatus);
}
