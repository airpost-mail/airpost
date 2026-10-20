import React from 'react';
import { ipcRenderer } from 'electron';
import ReactDOM from 'react-dom';
import { renderIntoDocument, Simulate } from 'react-dom/test-utils';
import { Account } from 'mailspring-exports';
import { AccountIMAPSettingsForm } from '../../src/features/onboarding/pages/account-settings-imap';
import { getPortalContainer } from '../../src/components/kit/portal';

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

// Opens a kit Select from its trigger and picks the item with this label.
function pick(trigger: Element, label: string) {
  Simulate.keyDown(trigger, { key: 'Enter' });
  const items = getPortalContainer().querySelectorAll(
    '[data-slot="select-content"][data-state="open"] [data-slot="select-item"]'
  );
  const item = Array.from(items).find((i) => i.textContent === label);
  expect(item).toBeDefined();
  Simulate.keyDown(item, { key: 'Enter' });
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
      // From a custom port, so every standard port is a change.
      const node = render(accountWith({ [`${protocol}_port`]: 2525 }), onFieldChange);
      pick(node.querySelector(`[id="settings.${protocol}_port"]`), `${port}`);
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

  describe('choosing a custom port', () => {
    it('sets port 0, which shows the custom port field', () => {
      const onFieldChange = jasmine.createSpy('onFieldChange');
      const node = render(accountWith(), onFieldChange);
      expect(node.querySelector('[id="settings.imap_port_custom"]')).toBe(null);
      pick(node.querySelector('[id="settings.imap_port"]'), 'Custom');
      expect(onFieldChange.calls[0].args[0].target).toEqual({ value: 0, id: 'settings.imap_port' });

      const custom = render(accountWith({ imap_port: 1143 }));
      const trigger = custom.querySelector('[id="settings.imap_port"]');
      expect(trigger.textContent).toBe('Custom');
      expect(
        (custom.querySelector('[id="settings.imap_port_custom"]') as HTMLInputElement).value
      ).toBe('1143');
    });
  });

  describe('the security select and the insecure SSL checkbox', () => {
    it('report changes under their field ids', () => {
      const onFieldChange = jasmine.createSpy('onFieldChange');
      const node = render(accountWith(), onFieldChange);
      const security = node.querySelector('[id="settings.smtp_security"]');
      expect(security.textContent).toBe('SSL / TLS');
      pick(security, 'STARTTLS');
      expect(onFieldChange.calls[0].args[0].target).toEqual({
        value: 'STARTTLS',
        id: 'settings.smtp_security',
      });

      const box = node.querySelector('[id="settings.imap_allow_insecure_ssl"]') as HTMLElement;
      expect(box.getAttribute('role')).toBe('checkbox');
      expect(box.getAttribute('aria-checked')).toBe('false');
      box.click();
      expect(onFieldChange.calls[1].args[0].target).toEqual({
        id: 'settings.imap_allow_insecure_ssl',
        type: 'checkbox',
        value: true,
        checked: true,
      });
    });
  });

  describe('saving none', () => {
    it('is blocked for a remote host until "send password unencrypted" is checked', () => {
      const account = accountWith({ imap_security: 'none', imap_port: 143 });
      let result = AccountIMAPSettingsForm.validateAccount(account);
      expect(result.errorFieldNames).toEqual(['imap_allow_cleartext']);
      expect(result.errorMessage).toBeTruthy();

      account.settings.imap_allow_cleartext = 'imap.example.com';
      result = AccountIMAPSettingsForm.validateAccount(account);
      expect(result.errorFieldNames).toEqual([]);
      expect(result.populated).toBe(true);
    });

    it('asks again when the host changes', () => {
      const account = accountWith({
        imap_security: 'none',
        imap_host: 'imap.other-isp.example',
        imap_allow_cleartext: 'imap.example.com',
      });
      expect(AccountIMAPSettingsForm.validateAccount(account).errorFieldNames).toEqual([
        'imap_allow_cleartext',
      ]);
      const box = render(account).querySelector('[id="settings.imap_allow_cleartext"]');
      expect(box.getAttribute('aria-checked')).toBe('false');

      // A bare `true` from before the confirmation named its host doesn't count either.
      (account.settings as any).imap_allow_cleartext = true;
      expect(AccountIMAPSettingsForm.validateAccount(account).errorFieldNames).toEqual([
        'imap_allow_cleartext',
      ]);
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
      const box = node.querySelector('[id="settings.smtp_allow_cleartext"]') as HTMLElement;
      expect(box).not.toBe(null);
      expect(box.getAttribute('aria-checked')).toBe('false');
      box.click();
      expect(onFieldChange.calls[0].args[0].target).toEqual({
        id: 'settings.smtp_allow_cleartext',
        value: 'smtp.example.com',
      });

      // Checked: unchecking withdraws the consent.
      const consented = render(
        accountWith({ smtp_security: 'none', smtp_allow_cleartext: 'smtp.example.com' }),
        onFieldChange
      );
      const checked = consented.querySelector(
        '[id="settings.smtp_allow_cleartext"]'
      ) as HTMLElement;
      expect(checked.getAttribute('aria-checked')).toBe('true');
      checked.click();
      expect(onFieldChange.calls[1].args[0].target.value).toBe(false);
    });
  });
});
