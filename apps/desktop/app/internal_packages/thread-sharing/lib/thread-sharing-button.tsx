import React from 'react';
import { localized, Thread } from 'mailspring-exports';
import { RetinaImg, BindGlobalCommands } from 'mailspring-component-kit';

const UNAVAILABLE = localized('Sharing threads needs a sharing server, and none is configured.');

// Sharing uploads the thread to a server that hosts the public page. There is no such server
// yet, so the button stays visible but disabled.
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
    return (
      <BindGlobalCommands
        commands={{
          'core:share-item-link': () => AppEnv.showErrorDialog(UNAVAILABLE),
          'core:copy-mailbox-link': () => this._onCopyMailboxLink(),
        }}
      >
        <button
          className="btn btn-toolbar thread-sharing-button"
          title={UNAVAILABLE}
          aria-label={UNAVAILABLE}
          disabled
          style={{ marginRight: 0 }}
        >
          <RetinaImg name="ic-toolbar-native-share.png" mode={RetinaImg.Mode.ContentIsMask} />
        </button>
      </BindGlobalCommands>
    );
  }
}
