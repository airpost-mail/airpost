import React, { Component } from 'react';
import ReactDOM from 'react-dom';
import { localized, Actions, Message, DraftEditingSession } from 'mailspring-exports';
import {
  Button,
  RetinaImg,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from 'mailspring-component-kit';
import moment from 'moment';

import SendRemindersPopover from './popover';
import { reminderDateFor, updateDraftReminderMetadata } from './utils';

interface SendRemindersComposerButtonProps {
  draft: Message;
  session: DraftEditingSession;
}
interface SendRemindersComposerButtonState {
  saving: boolean;
}

export default class SendRemindersComposerButton extends Component<
  SendRemindersComposerButtonProps,
  SendRemindersComposerButtonState
> {
  static displayName = 'SendRemindersComposerButton';

  static containerRequired = false;

  constructor(props: SendRemindersComposerButtonProps) {
    super(props);
    this.state = {
      saving: false,
    };
  }

  componentDidUpdate(prevProps: SendRemindersComposerButtonProps) {
    if (prevProps.draft !== this.props.draft && this.state.saving) {
      this.setState({ saving: false });
    }
  }

  shouldComponentUpdate(nextProps: SendRemindersComposerButtonProps) {
    return reminderDateFor(nextProps.draft) !== reminderDateFor(this.props.draft);
  }

  onSetReminder = (reminderDate: Date | null) => {
    const { draft, session } = this.props;
    this.setState({ saving: true });

    updateDraftReminderMetadata(session, {
      expiration: reminderDate,
      sentHeaderMessageId: draft.headerMessageId,
    });

    Actions.closePopover();
  };

  onClick = () => {
    const { draft } = this.props;
    const buttonRect = (ReactDOM.findDOMNode(this) as HTMLElement).getBoundingClientRect();
    Actions.openPopover(
      <SendRemindersPopover
        reminderDate={reminderDateFor(draft)}
        onRemind={(date) => this.onSetReminder(date)}
        onCancelReminder={() => this.onSetReminder(null)}
      />,
      { originRect: buttonRect, direction: 'up' }
    );
  };

  render() {
    let className = 'btn-send-reminder';
    let label: string;
    let content: React.ReactNode;

    if (this.state.saving) {
      label = localized('Saving reminder...');
      content = (
        <RetinaImg
          name="inline-loading-spinner.gif"
          mode={RetinaImg.Mode.ContentDark}
          style={{ width: 14, height: 14 }}
        />
      );
    } else {
      const reminderDate = reminderDateFor(this.props.draft);
      label = localized('Set Reminder');
      if (reminderDate) {
        className += ' btn-enabled';
        label = localized(`Reminder set for %@ from now`, moment(reminderDate).fromNow(true));
      }
      content = (
        <>
          <RetinaImg name="icon-composer-reminders.png" mode={RetinaImg.Mode.ContentIsMask} />
          <span>&nbsp;</span>
          <RetinaImg name="icon-composer-dropdown.png" mode={RetinaImg.Mode.ContentIsMask} />
        </>
      );
    }

    return (
      <Tooltip>
        <TooltipTrigger asChild>
          {/* The trigger's own data-slot would replace the button's. */}
          <Button
            data-slot="button"
            variant="toolbar"
            tabIndex={-1}
            className={className}
            aria-label={label}
            onClick={this.state.saving ? undefined : this.onClick}
          >
            {content}
          </Button>
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
    );
  }
}
