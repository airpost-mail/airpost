import React from 'react';
import { Thread, CapabilityStore } from 'mailspring-exports';
import { RetinaImg, BindGlobalCommands } from 'mailspring-component-kit';

// Sharing uploads the thread to a Tern server that hosts the public page. Until the capability
// store reports one, the button stays visible but disabled.
export default class ThreadSharingButton extends React.Component<{ items: any[]; thread: Thread }> {
  static displayName = 'ThreadSharingButton';

  static containerRequired = false;

  _onCopyMailboxLink = () => {
    navigator.clipboard
      .writeText(this.props.thread.getMailboxPermalink())
      .catch((err) => console.error('Failed to copy to clipboard:', err));
  };

  render() {
    if (this.props.items && this.props.items.length > 1) {
      return <span />;
    }
    const unavailable = CapabilityStore.unavailableMessage('thread-sharing');
    return (
      <BindGlobalCommands
        commands={{
          'core:share-item-link': () => AppEnv.showErrorDialog(unavailable),
          'core:copy-mailbox-link': () => this._onCopyMailboxLink(),
        }}
      >
        <button
          className="btn btn-toolbar thread-sharing-button"
          title={unavailable}
          aria-label={unavailable}
          disabled
          style={{ marginRight: 0 }}
        >
          <RetinaImg name="ic-toolbar-native-share.png" mode={RetinaImg.Mode.ContentIsMask} />
        </button>
      </BindGlobalCommands>
    );
  }
}
