import React from 'react';
import { localized, Actions, Message } from 'mailspring-exports';
import {
  Menu,
  RetinaImg,
  Button,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from 'mailspring-component-kit';
import TemplateStore from './store';

class TemplatePopover extends React.Component<{ headerMessageId: string }> {
  static displayName = 'TemplatePopover';

  unsubscribe?: () => void;

  state = {
    searchValue: '',
    templates: TemplateStore.items(),
  };

  componentDidMount() {
    this.unsubscribe = TemplateStore.listen(() => {
      this.setState({ templates: TemplateStore.items() });
    });
  }

  componentWillUnmount() {
    if (this.unsubscribe) {
      this.unsubscribe();
    }
  }

  _filteredTemplates() {
    const { searchValue, templates } = this.state;

    if (!searchValue.length) {
      return templates;
    }

    // Match both lines shown in each item the same way, so typing a word from
    // the middle of a name finds it just like a word from the middle of a subject.
    const query = searchValue.toLowerCase();
    return templates.filter((t) => {
      return (
        t.name.toLowerCase().includes(query) || (t.subject || '').toLowerCase().includes(query)
      );
    });
  }

  _onSearchValueChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    this.setState({ searchValue: event.target.value });
  };

  _onChooseTemplate = (template: ReturnType<typeof TemplateStore.items>[0]) => {
    Actions.insertTemplateId({
      templateId: template.id,
      headerMessageId: this.props.headerMessageId,
    });
    Actions.closePopover();
  };

  _onManageTemplates = () => {
    Actions.showTemplates();
  };

  _onNewTemplate = () => {
    Actions.createTemplate({ headerMessageId: this.props.headerMessageId });
  };

  render() {
    const filteredTemplates = this._filteredTemplates();

    const headerComponents = [
      <input
        type="text"
        autoFocus
        key="textfield"
        className="search"
        value={this.state.searchValue}
        onChange={this._onSearchValueChange}
      />,
    ];

    // note: these are using onMouseDown to avoid clearing focus in the composer (I think)
    const footerComponents = [
      <div className="item" key="new" onMouseDown={this._onNewTemplate}>
        {localized('Save Draft as Template...')}
      </div>,
      <div className="item" key="manage" onMouseDown={this._onManageTemplates}>
        {localized('Manage Templates...')}
      </div>,
    ];

    return (
      <Menu
        className="template-picker"
        headerComponents={headerComponents}
        footerComponents={footerComponents}
        items={filteredTemplates}
        itemKey={(item) => item.id}
        itemContent={(item) => (
          <div className="template">
            <div className="name">{item.name}</div>
            {item.subject ? <div className="subject">{item.subject}</div> : null}
          </div>
        )}
        onSelect={this._onChooseTemplate}
      />
    );
  }
}

class TemplatePicker extends React.Component<{
  headerMessageId: string;
  draft: Message;
}> {
  static displayName = 'TemplatePicker';

  _button = React.createRef<HTMLButtonElement>();

  _onClickButton = () => {
    if (!this._button.current) return;
    const buttonRect = this._button.current.getBoundingClientRect();
    Actions.openPopover(<TemplatePopover headerMessageId={this.props.headerMessageId} />, {
      originRect: buttonRect,
      direction: 'up',
    });
  };

  render() {
    if (this.props.draft.plaintext) {
      return <span />;
    }
    const label = localized('Quick Reply');
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            // TooltipTrigger's data-slot would otherwise replace the button's own.
            data-slot="button"
            ref={this._button}
            variant="toolbar"
            tabIndex={-1}
            className="btn-templates narrow pull-right"
            onClick={this._onClickButton}
            aria-label={label}
          >
            <RetinaImg
              url="tern-asset://composer-templates/assets/icon-composer-templates@2x.png"
              mode={RetinaImg.Mode.ContentIsMask}
              aria-hidden="true"
            />
            <RetinaImg
              name="icon-composer-dropdown.png"
              mode={RetinaImg.Mode.ContentIsMask}
              aria-hidden="true"
            />
          </Button>
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
    );
  }
}

export default TemplatePicker;
