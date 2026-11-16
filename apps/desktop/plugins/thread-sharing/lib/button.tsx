import React from 'react';
import { Thread, CapabilityStore, localized } from 'mailspring-exports';
import {
  BindGlobalCommands,
  IconButton,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from 'mailspring-component-kit';

// Sharing uploads the thread to a Airpost server that hosts the public page. The upload and the
// share popover aren't written yet, so the button stays disabled: with the capability store's
// reason until a server offers sharing, then with "not in this version yet".
// `thread` is the first selected item, so it is missing when nothing is selected.
export default class ThreadSharingButton extends React.Component<{
  items: any[];
  thread?: Thread | null;
}> {
  static displayName = 'ThreadSharingButton';

  static containerRequired = false;

  _onCopyMailboxLink = () => {
    const { thread } = this.props;
    if (!thread) return;
    navigator.clipboard
      .writeText(thread.getMailboxPermalink())
      .catch((err) => console.error('Failed to copy to clipboard:', err));
  };

  render() {
    if (this.props.items && this.props.items.length > 1) {
      return <span />;
    }
    const unavailable =
      CapabilityStore.unavailableMessage('thread-sharing') ??
      localized('Sharing threads comes in a later version.');
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
                label={localized('Share this thread')}
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
