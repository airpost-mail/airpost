import React from 'react';
import Rx, { Disposable } from 'rx-lite';
import { Flexbox } from '../flexbox';
import { localized } from '../../intl/localize';
import { Button } from '../kit/button';
import { TextField } from '../kit/text-field';
import { SettingsSelect } from '../settings-select';

import { Template, TemplateInstance } from './models';

const SOURCE_SELECT_NULL = 'NULL';

interface Item {
  value: string;
  name: string;
}
type SourceSelectProps = {
  value?: string;
  onChange: (...args: any[]) => any;
  options: Rx.Observable<Item[]> | Item[];
};
type SourceSelectState = {
  options: Item[];
};

class SourceSelect extends React.Component<SourceSelectProps, SourceSelectState> {
  static displayName = 'SourceSelect';

  _subscription?: Disposable;

  constructor(props: SourceSelectProps) {
    super(props);
    this.state = {
      options: [],
    };
  }

  componentDidMount() {
    this._setupValuesSubscription();
  }

  componentDidUpdate(prevProps: SourceSelectProps) {
    if (prevProps.options !== this.props.options) {
      this._setupValuesSubscription();
    }
  }

  componentWillUnmount() {
    if (this._subscription) {
      this._subscription.dispose();
    }
    this._subscription = undefined;
  }

  _setupValuesSubscription(props = this.props) {
    if (this._subscription) {
      this._subscription.dispose();
    }
    this._subscription = undefined;
    if (props.options instanceof (Rx.Observable as any)) {
      this._subscription = (props.options as Rx.Observable<Item[]>).subscribe((options) =>
        this.setState({ options })
      );
    } else {
      this.setState({ options: props.options as Item[] });
    }
  }

  _onChange = (key: string) => {
    const value = key === SOURCE_SELECT_NULL ? null : key;
    const option = this.state.options.find((o) => o.value === value);
    this.props.onChange({
      target: {
        value,
        name: option ? option.name : undefined,
      },
    });
  };

  render() {
    // No value is the blank SOURCE_SELECT_NULL option, which also clears the value.
    return (
      <SettingsSelect
        size="sm"
        value={this.props.value || SOURCE_SELECT_NULL}
        onChange={this._onChange}
        options={[
          [SOURCE_SELECT_NULL, ''],
          ...this.state.options.map(({ value, name }): [string, string] => [value, name]),
        ]}
      />
    );
  }
}

interface ScenarioEditorRowValue {
  templateKey: string;
  comparatorKey: string;
  value: string;
  valueName?: string;
}

interface ScenarioEditorRowProps {
  instance: ScenarioEditorRowValue;
  removable: boolean;
  templates: Template[];
  // coerceInstance drops comparatorKey for templates without comparators.
  onChange: (item: TemplateInstance) => void;
  onInsert: () => void;
  onRemove: () => void;
}
export default class ScenarioEditorRow extends React.Component<ScenarioEditorRowProps> {
  static displayName = 'ScenarioEditorRow';

  _onChangeValue = (event: { target: { value: string; name?: string } }) => {
    const instance = JSON.parse(JSON.stringify(this.props.instance));
    instance.value = event.target.value;
    // Enum options (folders, labels) are keyed by ids that can change on the server,
    // so the human-readable name is kept as a fallback for resolving the value later.
    // A DOM <input> reports `name` as "" rather than undefined, so test for content.
    if (event.target.name) {
      instance.valueName = event.target.name;
    } else {
      delete instance.valueName;
    }
    this.props.onChange(instance);
  };

  _onChangeComparator = (comparatorKey: string) => {
    const instance = JSON.parse(JSON.stringify(this.props.instance));
    instance.comparatorKey = comparatorKey;
    this.props.onChange(instance);
  };

  _onChangeTemplate = (templateKey: string) => {
    const instance = JSON.parse(JSON.stringify(this.props.instance));
    // The select only offers keys of this.props.templates.
    const newTemplate = this.props.templates.find((t) => t.key === templateKey)!;
    this.props.onChange(newTemplate.coerceInstance(instance));
  };

  _renderTemplateSelect() {
    return (
      <SettingsSelect
        size="sm"
        value={this.props.instance.templateKey}
        onChange={this._onChangeTemplate}
        options={this.props.templates.map(({ key, name }): [string, string] => [key, name])}
      />
    );
  }

  _renderComparator(template: Template) {
    const keys = Object.keys(template.comparators);
    if (keys.length === 0) {
      return false;
    }
    return (
      <SettingsSelect
        size="sm"
        value={this.props.instance.comparatorKey}
        onChange={this._onChangeComparator}
        options={keys.map((key): [string, string] => [key, template.comparators[key].name])}
      />
    );
  }

  _renderValue(template: Template) {
    if (template.type === Template.Type.Enum) {
      return (
        <SourceSelect
          value={this.props.instance.value}
          onChange={this._onChangeValue}
          // The Template constructor throws for an Enum without values.
          options={template.values!}
        />
      );
    }

    if (template.type === Template.Type.String || template.type === Template.Type.InputString) {
      return (
        <TextField size="sm" value={this.props.instance.value} onChange={this._onChangeValue} />
      );
    }
    return false;
  }

  _renderActions() {
    return (
      <div className="actions">
        {this.props.removable && (
          <Button variant="outline" aria-label={localized('Remove')} onClick={this.props.onRemove}>
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden>
              <path d="M1 5h8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
            </svg>
          </Button>
        )}
        <Button variant="outline" aria-label={localized('Add')} onClick={this.props.onInsert}>
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden>
            <path d="M5 1v8M1 5h8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
        </Button>
      </div>
    );
  }

  render() {
    const template = this.props.templates.find((t) => t.key === this.props.instance.templateKey);
    if (!template) {
      return (
        <span> Could not find template for instance key: {this.props.instance.templateKey}</span>
      );
    }
    return (
      <Flexbox direction="row" className="well-row">
        <div style={{ flex: 1, display: 'flex', flexWrap: 'wrap' }}>
          {this._renderTemplateSelect()}
          {this._renderComparator(template)}
          <span>{template.valueLabel}</span>
          {this._renderValue(template)}
        </div>
        {this._renderActions()}
      </Flexbox>
    );
  }
}
