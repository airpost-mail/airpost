import React from 'react';
import { Menu } from '../../components/menu';
import { TextField, TextFieldProps } from '../../components/kit/text-field';

type TypeaheadFreeInputProps = TextFieldProps & {
  suggestions: string[];
};

interface TypeaheadFreeInputState {
  focused: boolean;
}

export class TypeaheadFreeInput extends React.Component<
  TypeaheadFreeInputProps,
  TypeaheadFreeInputState
> {
  _menu = React.createRef<Menu & HTMLDivElement>();

  state = {
    focused: false,
  };

  onSelectSuggestion = (suggestion: string) => {
    this.props.onChange &&
      this.props.onChange({
        target: { value: suggestion },
        currentTarget: { value: suggestion },
      } as any);
  };

  render() {
    const { suggestions, ...rest } = this.props;
    const { focused } = this.state;

    let value = `${rest.value}`;

    const completions = suggestions.filter(
      (text) => text.toLowerCase().startsWith(value.toLowerCase()) && text !== value
    );

    // Adopt capitalization of the autocompletion
    if (completions[0] && completions[0].toLowerCase() === value) {
      value = completions[0];
    }

    return (
      <Menu
        ref={this._menu}
        items={focused ? completions : []}
        itemKey={(item) => item}
        itemContent={(item) => item}
        headerComponents={[
          <TextField
            key="input"
            onFocus={() => this.setState({ focused: true })}
            onBlur={() => this.setState({ focused: false })}
            onKeyDown={(e) => {
              // The input is one of the Menu's header components, so the Menu is mounted.
              this._menu.current!.onKeyDown(e);
            }}
            {...rest}
            value={value}
          />,
        ]}
        onSelect={this.onSelectSuggestion}
      />
    );
  }
}
