import React from 'react';
import ReactDOM from 'react-dom';
import { localized, Actions, Message } from 'mailspring-exports';
import { RetinaImg, Button } from 'mailspring-component-kit';
import OpenTrackingMessagePopover from './message-popover';
import { PLUGIN_ID } from './constants';

export default class OpenTrackingMessageStatus extends React.Component<{ message: Message }> {
  static displayName = 'OpenTrackingMessageStatus';

  static containerStyles = {
    paddingTop: 4,
  };

  onMouseDown = () => {
    const rect = (ReactDOM.findDOMNode(this) as HTMLElement).getBoundingClientRect();
    Actions.openPopover(
      <OpenTrackingMessagePopover
        message={this.props.message}
        openMetadata={this.props.message.metadataForPluginId(PLUGIN_ID)}
      />,
      { originRect: rect, direction: 'down' }
    );
  };

  _getDataFromMessage() {
    const metadata = this.props.message.metadataForPluginId(PLUGIN_ID);
    if (!metadata || metadata.open_count == null) {
      return {
        hasMetadata: false,
        openCount: null,
        opened: false,
      };
    }
    return {
      hasMetadata: true,
      openCount: metadata.open_count,
      opened: metadata.open_count > 0,
    };
  }

  render() {
    const { hasMetadata, openCount, opened } = this._getDataFromMessage();

    if (!hasMetadata) return false;
    const noun = openCount === 1 ? localized('Open') : localized('Opens');
    let count = openCount;
    if (openCount > 999) {
      count = '999+';
    }

    const text = opened ? `${count} ${noun.toLocaleLowerCase()}` : localized('No opens');
    const content = (
      <>
        <RetinaImg
          className={opened ? 'opened' : 'unopened'}
          style={{ position: 'relative', top: -1 }}
          url="tern-asset://open-tracking/assets/InMessage-opened@2x.png"
          mode={RetinaImg.Mode.ContentIsMask}
        />
        &nbsp;&nbsp;{text}
      </>
    );
    if (!opened) {
      return <span className="open-tracking-message-status unopened">{content}</span>;
    }
    return (
      <Button
        variant="ghost"
        size="sm"
        className="open-tracking-message-status opened"
        onMouseDown={this.onMouseDown}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            this.onMouseDown();
          }
        }}
      >
        {content}
      </Button>
    );
  }
}
