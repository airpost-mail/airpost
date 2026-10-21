import { ipcRenderer } from 'electron';
import React from 'react';
import { localized } from '../../../intl/localize';
import { Account } from '../../../data/models/account';
import CreatePageForForm from './create-page-for-form';
import FormField from '../form-field';
import { Checkbox } from '../../../components/kit/checkbox';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../../components/kit/select';
import {
  MailProtocol,
  cleartextConsentKey,
  hasCleartextConsent,
  isLoopbackHost,
  needsCleartextConsent,
  securityForPort,
} from '../connection-security';

const StandardIMAPPorts = [143, 993];
const StandardSMTPPorts = [25, 465, 587];
// The port select's Custom item; Radix Select values are non-empty strings.
const CUSTOM_PORT = 'custom';

interface AccountIMAPSettingsFormProps {
  account: Account;
  errorFieldNames: string[];
  submitting: boolean;
  onConnect: () => void;
  onFieldChange: (
    event: { target: { id: string; value: any; type?: string; checked?: boolean } },
    opts?: { afterSetState: () => void }
  ) => void;
  onFieldKeyPress: (e: React.KeyboardEvent) => void;
}

export class AccountIMAPSettingsForm extends React.Component<AccountIMAPSettingsFormProps> {
  static displayName = 'AccountIMAPSettingsForm';

  static submitLabel = () => {
    return localized('Connect Account');
  };

  static titleLabel = () => {
    return localized('Set up Account');
  };

  static subtitleLabel = () => {
    return localized('Complete the IMAP and SMTP settings below to connect your account.');
  };

  static validateAccount = (account: Account) => {
    let errorMessage: string | null = null;
    const errorFieldNames: string[] = [];

    if (!account.settings[`imap_username`] || !account.settings[`imap_password`]) {
      return { errorMessage, errorFieldNames, populated: false };
    }

    // Note: we explicitly don't check that an SMTP username / password
    // is provided because occasionally those gateways don't require them!

    for (const type of ['imap', 'smtp'] as MailProtocol[]) {
      if (!account.settings[`${type}_host`]) {
        return { errorMessage, errorFieldNames, populated: false };
      }
      if (!Number.isInteger(account.settings[`${type}_port`] / 1)) {
        errorMessage = localized('Please provide a valid port number.');
        errorFieldNames.push(`${type}_port`);
      }
      if (needsCleartextConsent(account.settings, type)) {
        errorMessage = localized(
          'Choose SSL / TLS or STARTTLS, or confirm that your password can be sent unencrypted.'
        );
        errorFieldNames.push(cleartextConsentKey(type));
      }
    }

    return { errorMessage, errorFieldNames, populated: true };
  };

  componentDidMount() {
    ipcRenderer.send('resize-window', { width: 900, height: 660 });
  }

  renderPortDropdown(protocol: MailProtocol) {
    if (!['imap', 'smtp'].includes(protocol)) {
      throw new Error(`Can't render port dropdown for protocol '${protocol}'`);
    }
    const {
      account: { settings },
      submitting,
      onFieldKeyPress,
      onFieldChange,
    } = this.props;

    const field = `${protocol}_port` as const;
    const values = protocol === 'imap' ? StandardIMAPPorts : StandardSMTPPorts;
    const isStandard = Number.isInteger(settings[field]) && values.includes(settings[field]);

    // When you change the port, automatically switch the security setting to
    // the standard for that port. Lots of people don't update that field and
    // are getting confused. No port implies cleartext: 143 and 25 get STARTTLS.
    const onPortChange = (event: { target: { value: string; id: string } }) => {
      const port = Number(event.target.value);
      const security = securityForPort(protocol, port);

      onFieldChange(
        { target: { value: port, id: event.target.id } },
        {
          afterSetState: () => {
            if (security && settings[`${protocol}_security`] !== security) {
              onFieldChange({ target: { value: security, id: `settings.${protocol}_security` } });
            }
          },
        }
      );
    };

    return (
      <span>
        <label htmlFor={`settings.${field}`}>{localized('Port')}:</label>
        <Select
          value={isStandard ? `${settings[field]}` : CUSTOM_PORT}
          disabled={submitting}
          onValueChange={(value) =>
            // Picking Custom sets port 0, which shows the custom port field.
            onPortChange({
              target: { value: value === CUSTOM_PORT ? '0' : value, id: `settings.${field}` },
            })
          }
        >
          <SelectTrigger id={`settings.${field}`}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {values.map((v) => (
              <SelectItem value={`${v}`} key={v}>
                {v}
              </SelectItem>
            ))}
            <SelectItem value={CUSTOM_PORT} key="custom">
              {localized('Custom')}
            </SelectItem>
          </SelectContent>
        </Select>
        {!isStandard && (
          <>
            <label htmlFor={`settings.${field}_custom`} className="sr-only">
              {localized('Custom Port')}
            </label>
            <input
              style={{
                width: 80,
                marginLeft: 6,
                height: 23,
              }}
              id={`settings.${field}_custom`}
              tabIndex={0}
              value={settings[field]}
              disabled={submitting}
              onKeyPress={onFieldKeyPress}
              onChange={(e) =>
                onPortChange({
                  target: { value: e.target.value, id: `settings.${field}` },
                })
              }
            />
          </>
        )}
      </span>
    );
  }

  renderSecurityDropdown(protocol: MailProtocol) {
    const {
      account: { settings },
      submitting,
      onFieldKeyPress,
      onFieldChange,
    } = this.props;
    const consentKey = cleartextConsentKey(protocol);
    const askConsent =
      settings[`${protocol}_security`] === 'none' && !isLoopbackHost(settings[`${protocol}_host`]);

    return (
      <div>
        <span>
          <label htmlFor={`settings.${protocol}_security`}>{localized('Security')}:</label>
          <Select
            value={settings[`${protocol}_security`]}
            disabled={submitting}
            onValueChange={(value) =>
              onFieldChange({ target: { value, id: `settings.${protocol}_security` } })
            }
          >
            <SelectTrigger id={`settings.${protocol}_security`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="SSL / TLS" key="SSL / TLS">
                SSL / TLS
              </SelectItem>
              <SelectItem value="STARTTLS" key="STARTTLS">
                STARTTLS
              </SelectItem>
              <SelectItem value="none" key="none">
                {localized('None')}
              </SelectItem>
            </SelectContent>
          </Select>
        </span>
        <span style={{ paddingLeft: '20px', paddingTop: '10px' }}>
          <Checkbox
            id={`settings.${protocol}_allow_insecure_ssl`}
            disabled={submitting}
            checked={settings[`${protocol}_allow_insecure_ssl`] || false}
            onKeyDown={onFieldKeyPress}
            onCheckedChange={(checked) =>
              onFieldChange({
                target: {
                  id: `settings.${protocol}_allow_insecure_ssl`,
                  type: 'checkbox',
                  value: checked === true,
                  checked: checked === true,
                },
              })
            }
          />
          <label htmlFor={`settings.${protocol}_allow_insecure_ssl`} className="checkbox">
            {localized('Allow insecure SSL')}
          </label>
        </span>
        {askConsent && (
          <div style={{ paddingTop: '6px' }}>
            <Checkbox
              id={`settings.${consentKey}`}
              disabled={submitting}
              checked={hasCleartextConsent(settings, protocol)}
              onKeyDown={onFieldKeyPress}
              onCheckedChange={(checked) =>
                onFieldChange({
                  target: {
                    id: `settings.${consentKey}`,
                    value: checked === true ? `${settings[`${protocol}_host`]}`.trim() : false,
                  },
                })
              }
            />
            <label htmlFor={`settings.${consentKey}`} className="checkbox">
              {localized('Send password unencrypted')}
            </label>
          </div>
        )}
      </div>
    );
  }

  renderFieldsForType(type: MailProtocol) {
    return (
      <div>
        <FormField field={`settings.${type}_host`} title={localized('Server')} {...this.props} />
        <div style={{ textAlign: 'left' }}>
          {this.renderPortDropdown(type)}
          {this.renderSecurityDropdown(type)}
        </div>
        <FormField
          field={`settings.${type}_username`}
          title={localized('Username')}
          {...this.props}
        />
        <FormField
          field={`settings.${type}_password`}
          title={localized('Password')}
          type="password"
          {...this.props}
        />
        {type === 'imap' && (
          <FormField
            field={`settings.container_folder`}
            title={localized('Custom Container Folder')}
            {...this.props}
          />
        )}
      </div>
    );
  }

  render() {
    return (
      <div className="twocol">
        <div className="col">
          <div className="col-heading">{localized('Incoming Mail')} (IMAP):</div>
          {this.renderFieldsForType('imap')}
        </div>
        <div className="col">
          <div className="col-heading">{localized('Outgoing Mail')} (SMTP):</div>
          {this.renderFieldsForType('smtp')}
        </div>
      </div>
    );
  }
}

export default CreatePageForForm(AccountIMAPSettingsForm);
