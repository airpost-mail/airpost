import React from 'react';
import { Comparator, Template, TemplateInstance } from './models';
import ScenarioEditorRow from './row';

type ScenarioEditorProps = {
  instances?: any[];
  className?: string;
  onChange?: (...args: any[]) => any;
  templates?: any[];
};
type ScenarioEditorState = {
  collapsed: boolean;
};
/**
The ScenarioEditor takes an array of ScenarioTemplate objects which define the
scenario value space. Each ScenarioTemplate defines a `key` and it's valid
`comparators` and `values`. The ScenarioEditor gives the user the option to
create and combine instances of different templates to create a scenario.

For example:

  Scenario Space:
   - ScenarioFactory("user-name", "The name of the user")
      + valueType: String
      + comparators: "contains", "starts with", etc.
    - SecnarioFactor("profession", "The profession of the user")
      + valueType: Enum
      + comparators: 'is'

  Scenario Value:
    [{
      'key': 'user-name'
      'comparator': 'contains'
      'value': 'Ben'
    },{
      'key': 'profession'
      'comparator': 'is'
      'value': 'Engineer'
    }]
*/

export default class ScenarioEditor extends React.Component<
  ScenarioEditorProps,
  ScenarioEditorState
> {
  static displayName = 'ScenarioEditor';

  static Template = Template;
  static Comparator = Comparator;

  constructor(props: ScenarioEditorProps) {
    super(props);
    this.state = {
      collapsed: true,
    };
  }

  _performChange(block: (instances: TemplateInstance[]) => void) {
    const instances: TemplateInstance[] = JSON.parse(JSON.stringify(this.props.instances));
    block(instances);
    this.props.onChange?.(instances);
  }

  _templates(): Template[] {
    if (!this.props.templates) {
      throw new Error('ScenarioEditor: this needs templates');
    }
    return this.props.templates;
  }

  _onRemoveRule = (idx: number) => {
    this._performChange((instances) => {
      if (instances.length > 1) {
        instances.splice(idx, 1);
      }
    });
  };

  _onInsertRule = () => {
    this._performChange((instances) => {
      instances.push(this._templates()[0].createDefaultInstance());
    });
  };

  _onChangeRowValue = (newInstance: TemplateInstance, idx: number) => {
    this._performChange((instances) => {
      instances[idx] = newInstance;
    });
  };

  render() {
    return (
      <div className={this.props.className}>
        {(this.props.instances || []).map((instance, idx, instances) => (
          <ScenarioEditorRow
            key={idx}
            instance={instance}
            removable={instances.length > 1}
            templates={this._templates()}
            onRemove={() => this._onRemoveRule(idx)}
            onInsert={() => this._onInsertRule()}
            onChange={(updatedInstance) => this._onChangeRowValue(updatedInstance, idx)}
          />
        ))}
      </div>
    );
  }
}
