import React from 'react';
import { ipcRenderer } from 'electron';
import ReactDOM from 'react-dom';
import { renderIntoDocument, Simulate } from 'react-dom/test-utils';
import { Account } from 'mailspring-exports';
import { AccountIMAPSettingsForm } from '../lib/page-account-settings-imap';

function accountWith(settings = {}) {
  return new Account({
    name: 'Test',
    emailAddress: 'user@example.com',
    provider: 'imap',
    settings: {
      imap_host: 'imap.example.com',
      imap_port: 993,
      imap_username: 'user',
      imap_password: 'secret',
      imap_security: 'SSL / TLS',
      smtp_host: 'smtp.example.com',
      smtp_port: 465,
      smtp_username: 'user',
      smtp_password: 'secret',
      smtp_security: 'SSL / TLS',
      ...settings,
    },
  } as any);
}

function render(account: Account, onFieldChange = jasmine.createSpy('onFieldChange')) {
  const component = renderIntoDocument(
    <AccountIMAPSettingsForm
      account={account}
      errorFieldNames={[]}
      submitting={false}
      onConnect={() => {}}
      onFieldChange={onFieldChange}
      onFieldKeyPress={() => {}}
    />
  ) as any;
  return ReactDOM.findDOMNode(component) as HTMLElement;
}

describe('AccountIMAPSettingsForm', function imapSettingsFormSpecs() {
  beforeEach(() => {
    spyOn(ipcRenderer, 'send');
  });

  describe('changing the port', () => {
    // The form applies the security change after the port is in state.
    function securityAfterPortChange(protocol: 'imap' | 'smtp', port: number) {
      const onFieldChange = jasmine.createSpy('onFieldChange');
      const node = render(accountWith(), onFieldChange);
      const select = node.querySelector(`[id="settings.${protocol}_port"]`);
      Simulate.change(select, {
        target: { value: `${port}`, id: `settings.${protocol}_port` },
      } as any);
      expect(onFieldChange.calls[0].args[0].target).toEqual({
        value: port,
        id: `settings.${protocol}_port`,
      });
      onFieldChange.calls[0].args[1].afterSetState();
      const change = onFieldChange.calls
        .slice(1)
        .find((c) => c.args[0].target.id === `settings.${protocol}_security`);
      return change ? change.args[0].target.value : null;
    }

    it('picks STARTTLS for 143 and 25, not none', () => {
      expect(securityAfterPortChange('imap', 143)).toEqual('STARTTLS');
      expect(securityAfterPortChange('smtp', 25)).toEqual('STARTTLS');
    });

    it('picks the standard security for the other standard ports', () => {
      expect(securityAfterPortChange('smtp', 587)).toEqual('STARTTLS');
      // Already SSL / TLS, so nothing changes.
      expect(securityAfterPortChange('imap', 993)).toBe(null);
      expect(securityAfterPortChange('smtp', 465)).toBe(null);
    });
  });

  describe('saving none', () => {
    it('is blocked for a remote host until "send password unencrypted" is checked', () => {
      const account = accountWith({ imap_security: 'none', imap_port: 143 });
      let result = AccountIMAPSettingsForm.validateAccount(account);
      expect(result.errorFieldNames).toEqual(['imap_allow_cleartext']);
      expect(result.errorMessage).toBeTruthy();

      account.settings.imap_allow_cleartext = true;
      result = AccountIMAPSettingsForm.validateAccount(account);
      expect(result.errorFieldNames).toEqual([]);
      expect(result.populated).toBe(true);
    });

    it('gates SMTP the same way', () => {
      const account = accountWith({ smtp_security: 'none', smtp_port: 25 });
      expect(AccountIMAPSettingsForm.validateAccount(account).errorFieldNames).toEqual([
        'smtp_allow_cleartext',
      ]);
    });

    it('needs no confirmation for localhost (Proton Bridge)', () => {
      for (const host of ['127.0.0.1', 'localhost']) {
        const account = accountWith({
          imap_host: host,
          imap_security: 'none',
          smtp_host: host,
          smtp_security: 'none',
        });
        const result = AccountIMAPSettingsForm.validateAccount(account);
        expect(result.errorFieldNames).toEqual([]);
        expect(result.populated).toBe(true);
        expect(render(account).querySelector('[id="settings.imap_allow_cleartext"]')).toBe(null);
      }
    });

    it('shows the checkbox only when none is selected for a remote host', () => {
      expect(render(accountWith()).querySelector('[id="settings.imap_allow_cleartext"]')).toBe(
        null
      );
      const onFieldChange = jasmine.createSpy('onFieldChange');
      const node = render(accountWith({ smtp_security: 'none' }), onFieldChange);
      expect(node.querySelector('[id="settings.imap_allow_cleartext"]')).toBe(null);
      const box = node.querySelector('[id="settings.smtp_allow_cleartext"]') as HTMLInputElement;
      expect(box).not.toBe(null);
      expect(box.checked).toBe(false);
      Simulate.change(box, { target: { id: box.id, type: 'checkbox', checked: true } } as any);
      expect(onFieldChange.calls[0].args[0].target.checked).toBe(true);
    });
  });
});
