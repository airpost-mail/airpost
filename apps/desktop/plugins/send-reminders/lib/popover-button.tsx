import React, { Component } from 'react';
import ReactDOM from 'react-dom';
import classNames from 'classnames';
import { localized, Actions, DatabaseStore, Message, Thread } from 'mailspring-exports';
import { IconButton } from 'mailspring-component-kit';
import SendRemindersPopover from './popover';
import { updateReminderMetadata, reminderDateFor } from './utils';

export default class SendRemindersPopoverButton extends Component<{
  className?: string;
  thread: Thread;
  direction: string;
  getBoundingClientRect: (me: SendRemindersPopoverButton) => ClientRect;
}> {
  static displayName = 'SendRemindersPopoverButton';

  static defaultProps = {
    direction: 'down',
    getBoundingClientRect: (inst: SendRemindersPopoverButton) =>
      (ReactDOM.findDOMNode(inst) as HTMLElement).getBoundingClientRect(),
  };

  onSetReminder = async (expiration: Date | null) => {
    const { thread } = this.props;

    // get the messages on the thread and find the last one received.
    // we need to identify the message which will show the reminder
    DatabaseStore.findAll<Message>(Message, { threadId: thread.id }).then((messages) => {
      const lastSent = messages.reverse().find((m) => m.isFromMe());
      const metadata = expiration
        ? {
            expiration,
            shouldNotify: false,
            sentHeaderMessageId: lastSent ? lastSent.headerMessageId : null,
            lastReplyTimestamp: new Date(thread.lastMessageReceivedTimestamp).getTime() / 1000,
          }
        : {};

      updateReminderMetadata(thread, metadata);

      Actions.closePopover();
    });
  };

  onClick = (event: React.MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    const { direction, thread, getBoundingClientRect } = this.props;
    const reminderDate = reminderDateFor(thread);
    const buttonRect = getBoundingClientRect(this);
    Actions.openPopover(
      <SendRemindersPopover
        reminderDate={reminderDate}
        onRemind={(date) => this.onSetReminder(date)}
        onCancelReminder={() => this.onSetReminder(null)}
      />,
      { originRect: buttonRect, direction }
    );
  };

  render() {
    const { className, thread } = this.props;
    const reminderDate = reminderDateFor(thread);
    const title = reminderDate ? localized('Edit Reminder') : localized('Set Reminder');
    return (
      <IconButton
        variant="toolbar"
        tabIndex={-1}
        label={title}
        icon="ic-toolbar-native-reminder.png"
        className={classNames('send-reminders-toolbar-button', className)}
        style={{ order: -109 }}
        onClick={this.onClick}
      />
    );
  }
}
