import React from 'react';
import { Switch as KitSwitch } from './kit/switch';

/* Public: A small React component which renders as a horizontal on/off switch.
   Provide it with `onChange` and `checked` props just like a checkbox:

  ```
  <Switch onChange={this._onToggleChecked} checked={this.state.form.isChecked} />
  ```

  The UI kit's Switch (src/components/kit/switch.tsx) behind this export's props. It keeps the
  `slide-switch`, `active` and `handle` classes; the root is a <button role="switch">.
*/
type SwitchProps = {
  checked?: boolean;
  onChange: (...args: any[]) => any;
  className?: string;
  label?: string;
  labelledBy?: string;
};

const Switch: React.FunctionComponent<SwitchProps> = (props) => (
  // Always controlled: without `checked` it shows off and stays off, as before.
  <KitSwitch
    className={props.className}
    checked={!!props.checked}
    label={props.label}
    labelledBy={props.labelledBy}
    onChange={props.onChange}
  />
);
Switch.displayName = 'Switch';

export default Switch;
