import React from 'react';
import ReactDOM from 'react-dom';
import { localized, Actions, Message } from 'mailspring-exports';
import {
  RetinaImg,
  Button,
  Tooltip,
  TooltipTrigger,
  TooltipContent,
} from 'mailspring-component-kit';
import OpenTrackingMessagePopover from './message-popover';
import { PLUGIN_ID } from './constants';

export default class OpenTrackingIcon extends React.Component<{
  /** A thread list item: the thread with its messages attached. Only `__messages` is read. */
  thread: { __messages: Message[] };
}> {
  static displayName = 'OpenTrackingIcon';

  onMouseDown = () => {
    const rect = (ReactDOM.findDOMNode(this) as HTMLElement).getBoundingClientRect();
    const { message } = this._getDataFromThread();
    // Only rendered as a button when the last message has metadata.
    if (!message) return;
    Actions.openPopover(
      <OpenTrackingMessagePopover
        message={message}
        openMetadata={message.metadataForPluginId(PLUGIN_ID)}
      />,
      { originRect: rect, direction: 'down' }
    );
  };

  _getDataFromThread() {
    const messages = this.props.thread.__messages || [];

    let lastMessage = null;
    for (let i = messages.length - 1; i >= 0; i--) {
      if (!messages[i].draft) {
        lastMessage = messages[i];
        break;
      }
    }

    if (!lastMessage) {
      return {
        message: null,
        opened: false,
        openCount: null,
        hasMetadata: false,
      };
    }

    const lastMessageMeta = lastMessage.metadataForPluginId(PLUGIN_ID);
    const hasMetadata = lastMessageMeta != null && lastMessageMeta.open_count != null;

    return {
      message: lastMessage,
      opened: hasMetadata && lastMessageMeta.open_count > 0,
      openCount: hasMetadata ? lastMessageMeta.open_count : null,
      hasMetadata: hasMetadata,
    };
  }

  render() {
    const { hasMetadata, opened, openCount } = this._getDataFromThread();

    if (!hasMetadata) return <span style={{ width: '19px' }} />;
    const noun = openCount === 1 ? localized('Open') : localized('Opens');
    const title = opened
      ? `${openCount} ${noun.toLocaleLowerCase()}`
      : localized('This message has not been opened');
    const image = (
      <RetinaImg
        className={opened ? 'opened' : 'unopened'}
        url="tern-asset://open-tracking/assets/icon-tracking-opened@2x.png"
        mode={RetinaImg.Mode.ContentIsMask}
        aria-hidden="true"
      />
    );
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          {opened ? (
            // data-slot is set again because the trigger's own data-slot would replace it.
            <Button
              variant="ghost"
              data-slot="button"
              className="open-tracking-icon"
              aria-label={title}
              onMouseDown={this.onMouseDown}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  this.onMouseDown();
                }
              }}
            >
              {image}
            </Button>
          ) : (
            <div className="open-tracking-icon" aria-label={title}>
              {image}
            </div>
          )}
        </TooltipTrigger>
        <TooltipContent>{title}</TooltipContent>
      </Tooltip>
    );
  }
}
