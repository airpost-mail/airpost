import React from 'react';
import { localized } from 'mailspring-exports';

// Uploading a picture needs a server to host it, and there is none yet, so a custom picture is
// given as a URL.
export default class SignaturePhotoPicker extends React.Component<{
  data: any;
  resolvedURL: string;
  onChange: (e: { target: { value: string; id?: string } }) => void;
}> {
  render() {
    const { data, resolvedURL } = this.props;

    let source = data.photoURL || '';
    if (!['gravatar', 'company', ''].includes(source)) {
      source = 'custom';
    }

    // A picture uploaded to Mailspring's servers gets a Remove button instead of an editable URL.
    const isMailspringURL = resolvedURL && resolvedURL.includes('getmailspring.com');

    return (
      <div className="field photo-picker">
        <label htmlFor="photoURL">Picture</label>
        <div style={{ display: 'flex' }}>
          <div className="photo-options">
            <select
              id="photoURL"
              tabIndex={-1}
              value={source || ''}
              onChange={this.props.onChange}
              style={{ display: 'block' }}
            >
              <option value="">{localized('None')}</option>
              <option value="gravatar">{localized('Gravatar Profile Photo')}</option>
              <option value="company">{localized('Company / Domain Logo')}</option>
              <option disabled>──────────</option>
              <option value="custom">{localized('Custom Image…')}</option>
            </select>
            {source === 'custom' &&
              (isMailspringURL ? (
                <a
                  className="btn"
                  onClick={() => this.props.onChange({ target: { value: '', id: 'photoURL' } })}
                >
                  {localized('Remove')}
                </a>
              ) : (
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
              ))}
          </div>
        </div>
      </div>
    );
  }
}
