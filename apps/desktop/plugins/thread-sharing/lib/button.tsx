import React from 'react';
import { Thread, CapabilityStore } from 'mailspring-exports';
import {
  BindGlobalCommands,
  IconButton,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from 'mailspring-component-kit';

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
        <Tooltip>
          {/* The span is the trigger: a disabled button takes no pointer events, and the
              tooltip says why it is disabled. */}
          <TooltipTrigger asChild>
            <span className="thread-sharing-button">
              <IconButton
                variant="toolbar"
                icon="ic-toolbar-native-share.png"
                label={unavailable}
                tooltip={false}
                disabled
                style={{ marginRight: 0 }}
              />
            </span>
          </TooltipTrigger>
          <TooltipContent>{unavailable}</TooltipContent>
        </Tooltip>
      </BindGlobalCommands>
    );
  }
}
