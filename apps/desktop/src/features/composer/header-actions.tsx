import React from 'react';
import { localized, Actions } from 'mailspring-exports';
import { RovingTabIndexToolbar } from 'mailspring-component-kit';
import { Button } from '../../components/kit/button';
import { IconButton } from '../../components/kit/icon-button';
import Fields from './fields';

interface ComposerHeaderActionsProps {
  headerMessageId: string;
  enabledFields: string[];
  onShowAndFocusField: (f: string) => void;
}

export default class ComposerHeaderActions extends React.Component<ComposerHeaderActionsProps> {
  static displayName = 'ComposerHeaderActions';

  _onPopoutComposer = () => {
    Actions.composePopoutDraft(this.props.headerMessageId);
  };

  render() {
    const items = [];
    const showable = [
      { field: Fields.Cc, className: 'show-cc', key: 'cc', label: localized('Cc') },
      { field: Fields.Bcc, className: 'show-bcc', key: 'bcc', label: localized('Bcc') },
      {
        field: Fields.ReplyTo,
        className: 'show-reply-to',
        key: 'replyTo',
        label: localized('Reply-To'),
      },
      {
        field: Fields.Subject,
        className: 'show-subject',
        key: 'subject',
        label: localized('Subject'),
      },
    ];

    for (const { field, className, key, label } of showable) {
      if (this.props.enabledFields.includes(field)) continue;
      items.push(
        <Button
          variant="link"
          size="sm"
          className={`action ${className}`}
          key={key}
          tabIndex={-1}
          onClick={() => this.props.onShowAndFocusField(field)}
        >
          {label}
        </Button>
      );
    }

    if (!AppEnv.isComposerWindow()) {
      items.push(
        <IconButton
          className="action show-popout"
          key="popout"
          tabIndex={-1}
          icon="composer-popout.png"
          label={localized('Popout composer…')}
          onClick={this._onPopoutComposer}
        />
      );
    }

    return (
      <RovingTabIndexToolbar
        label={localized('Composer header actions')}
        className="composer-header-actions"
      >
        {items}
      </RovingTabIndexToolbar>
    );
  }
}
