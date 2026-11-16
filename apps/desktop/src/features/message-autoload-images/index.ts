import ComponentRegistry from '../../packages/component-registry';
import * as ExtensionRegistry from '../../packages/extension-registry';

import AutoloadImagesExtension from './extension';
import AutoloadImagesHeader from './header';

/*
All packages must export a basic object that has at least the following 3
methods:

1. `activate` - Actions to take once the package gets turned on.
Pre-enabled packages get activated when Airpost starts. They can also be
activated manually by a user.

2. `deactivate` - Actions to take when a package gets turned off. This can
happen when a user manually disables a package.
*/
export function activate() {
  // Register Message List Actions we provide globally
  ExtensionRegistry.MessageView.register(AutoloadImagesExtension);
  ComponentRegistry.register(AutoloadImagesHeader, {
    role: 'message:BodyHeader',
  });
}

export function deactivate() {
  ExtensionRegistry.MessageView.unregister(AutoloadImagesExtension);
  ComponentRegistry.unregister(AutoloadImagesHeader);
}
