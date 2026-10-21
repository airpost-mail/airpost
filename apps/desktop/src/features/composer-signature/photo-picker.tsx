import React from 'react';
import { localized } from '../../intl/localize';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from '../../components/kit/select';

// Radix Select can't hold an empty value, so "None" goes through this sentinel.
const NONE = '__none__';

// Uploading a picture needs a server to host it, and there is none yet, so a custom picture is
// given as a URL.
export default class SignaturePhotoPicker extends React.Component<{
  data: any;
  onChange: (e: { target: { value: string; id: string } }) => void;
}> {
  _onSourceChange = (value: string) => {
    this.props.onChange({ target: { id: 'photoURL', value: value === NONE ? '' : value } });
  };

  render() {
    const { data } = this.props;

    let source = data.photoURL || '';
    // 'company' is a removed domain-logo option that saved signature data can still hold.
    if (source === 'company') {
      source = '';
    } else if (!['gravatar', ''].includes(source)) {
      source = 'custom';
    }

    return (
      <div className="field photo-picker">
        <label htmlFor="photoURL">Picture</label>
        <div style={{ display: 'flex' }}>
          <div className="photo-options">
            <Select value={source || NONE} onValueChange={this._onSourceChange}>
              <SelectTrigger id="photoURL" tabIndex={-1}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE}>{localized('None')}</SelectItem>
                <SelectItem value="gravatar">{localized('Gravatar Profile Photo')}</SelectItem>
                <SelectSeparator />
                <SelectItem value="custom">{localized('Custom Image…')}</SelectItem>
              </SelectContent>
            </Select>
            {source === 'custom' && (
              <>
                <label htmlFor="photoURL" className="sr-only">
                  {localized('Photo URL')}
                </label>
                <input
                  type="url"
                  id="photoURL"
                  placeholder="http://"
                  value={data.photoURL === 'custom' ? '' : data.photoURL}
                  onChange={this.props.onChange}
                />
              </>
            )}
          </div>
        </div>
      </div>
    );
  }
}
