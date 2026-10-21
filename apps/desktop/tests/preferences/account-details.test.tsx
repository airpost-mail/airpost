import React from 'react';
import ReactDOM from 'react-dom';
import { renderIntoDocument } from 'react-dom/test-utils';
import { Account } from '../../src/data/models/account';

import PreferencesAccountDetails from '../../src/features/preferences/tabs/accounts/details';

const makeComponent = (props: any = {}) => {
  return renderIntoDocument(<PreferencesAccountDetails {...(props as any)} />) as any;
};

const account = new Account({
  id: 1,
  name: 'someone',
  emailAddress: 'someone@example.com',
  aliases: [],
  defaultAlias: null,
});

describe('PreferencesAccountDetails', function preferencesAccountDetails() {
  beforeEach(() => {
    this.account = account;
    this.onAccountUpdated = jasmine.createSpy('onAccountUpdated');
    this.component = makeComponent({ account, onAccountUpdated: this.onAccountUpdated });
    spyOn(this.component, 'setState');
  });

  function assertAccountState(actual, expected) {
    for (const key of Object.keys(expected)) {
      expect(actual.account[key]).toEqual(expected[key]);
    }
  }

  describe('_makeAlias', () => {
    it('returns correct alias when empty string provided', () => {
      const alias = this.component._makeAlias('', this.account);
      expect(alias).toEqual('someone <someone@example.com>');
    });

    it('returns correct alias when only the name provided', () => {
      const alias = this.component._makeAlias('Chad', this.account);
      expect(alias).toEqual('Chad <someone@example.com>');
    });

    it('returns correct alias when email provided', () => {
      const alias = this.component._makeAlias('keith@example.com', this.account);
      expect(alias).toEqual('someone <keith@example.com>');
    });

    it('returns correct alias if name and email provided', () => {
      const alias = this.component._makeAlias('Donald donald@example.com', this.account);
      expect(alias).toEqual('Donald <donald@example.com>');
    });

    it('returns correct alias if alias provided', () => {
      const alias = this.component._makeAlias('Donald <donald@example.com>', this.account);
      expect(alias).toEqual('Donald <donald@example.com>');
    });
  });

  describe('_setState', () => {
    it('sets the correct state', () => {
      this.component._setState({ aliases: ['something'] });
      assertAccountState(this.component.setState.calls[0].args[0], { aliases: ['something'] });
    });
  });

  describe('_onDefaultAliasSelected', () => {
    it('sets the default alias correctly when set to None', () => {
      this.component._onDefaultAliasSelected('None');
      assertAccountState(this.component.setState.calls[0].args[0], { defaultAlias: null });
    });

    it('sets the default alias correctly when set to any value', () => {
      this.component._onDefaultAliasSelected('my alias');
      assertAccountState(this.component.setState.calls[0].args[0], { defaultAlias: 'my alias' });
    });
  });

  describe('Save drafts to server', () => {
    const withSettings = (settings: Record<string, any>) =>
      new Account({
        id: 'acct-drafts',
        name: 'someone',
        emailAddress: 'someone@example.com',
        settings: { imap_host: 'imap.example.com', ...settings },
      } as any);

    const renderedSwitch = (component: any) =>
      Array.from(
        (ReactDOM.findDOMNode(component) as HTMLElement).querySelectorAll('[role="switch"]')
      ).find((el) => el.getAttribute('aria-label') === 'Save drafts to server') as HTMLElement;

    it('is on when the account has no setting', () => {
      const component = makeComponent({ account: withSettings({}), onAccountUpdated() {} });
      expect(renderedSwitch(component).getAttribute('aria-checked')).toBe('true');
    });

    it('is off when the setting is false', () => {
      const component = makeComponent({
        account: withSettings({ save_drafts_to_server: false }),
        onAccountUpdated() {},
      });
      expect(renderedSwitch(component).getAttribute('aria-checked')).toBe('false');
    });

    it('saves the setting with the account, keeping its other settings', () => {
      const onAccountUpdated = jasmine.createSpy('onAccountUpdated');
      const component = makeComponent({ account: withSettings({}), onAccountUpdated });
      component._onSaveDraftsToServerChanged(false);
      expect(onAccountUpdated).toHaveBeenCalled();
      const updated = onAccountUpdated.calls[0].args[1] as Account;
      expect(updated.settings.save_drafts_to_server).toBe(false);
      expect(updated.settings.imap_host).toBe('imap.example.com');
    });
  });

  describe('alias handlers', () => {
    beforeEach(() => {
      this.currentAlias = 'juan <blah@example>';
      this.newAlias = 'some <alias@example.com>';
      this.account.aliases = [this.currentAlias];
      this.component = makeComponent({
        account: this.account,
        onAccountUpdated: this.onAccountUpdated,
      });
      spyOn(this.component, '_makeAlias').andCallFake((alias) => alias);
      spyOn(this.component, 'setState');
    });
    describe('_onAccountAliasCreated', () => {
      it('creates alias correctly', () => {
        this.component._onAccountAliasCreated(this.newAlias);
        assertAccountState(this.component.setState.calls[0].args[0], {
          aliases: [this.currentAlias, this.newAlias],
        });
      });
    });

    describe('_onAccountAliasUpdated', () => {
      it('updates alias correctly when no default alias present', () => {
        this.component._onAccountAliasUpdated(this.newAlias, this.currentAlias, 0);
        assertAccountState(this.component.setState.calls[0].args[0], { aliases: [this.newAlias] });
      });

      it('updates alias correctly when default alias present and it is being updated', () => {
        this.account.defaultAlias = this.currentAlias;
        this.component = makeComponent({
          account: this.account,
          onAccountUpdated: this.onAccountUpdated,
        });
        spyOn(this.component, '_makeAlias').andCallFake((alias) => alias);
        spyOn(this.component, 'setState');

        this.component._onAccountAliasUpdated(this.newAlias, this.currentAlias, 0);
        assertAccountState(this.component.setState.calls[0].args[0], {
          aliases: [this.newAlias],
          defaultAlias: this.newAlias,
        });
      });

      it('updates alias correctly when default alias present and it is not being updated', () => {
        this.account.defaultAlias = this.currentAlias;
        this.account.aliases.push('otheralias');
        this.component = makeComponent({
          account: this.account,
          onAccountUpdated: this.onAccountUpdated,
        });
        spyOn(this.component, '_makeAlias').andCallFake((alias) => alias);
        spyOn(this.component, 'setState');

        this.component._onAccountAliasUpdated(this.newAlias, 'otheralias', 1);
        assertAccountState(this.component.setState.calls[0].args[0], {
          aliases: [this.currentAlias, this.newAlias],
          defaultAlias: this.currentAlias,
        });
      });
    });

    describe('_onAccountAliasRemoved', () => {
      it('removes alias correctly when no default alias present', () => {
        this.component._onAccountAliasRemoved(this.currentAlias, 0);
        assertAccountState(this.component.setState.calls[0].args[0], { aliases: [] });
      });

      it('removes alias correctly when default alias present and it is being removed', () => {
        this.account.defaultAlias = this.currentAlias;
        this.component = makeComponent({
          account: this.account,
          onAccountUpdated: this.onAccountUpdated,
        });
        spyOn(this.component, '_makeAlias').andCallFake((alias) => alias);
        spyOn(this.component, 'setState');

        this.component._onAccountAliasRemoved(this.currentAlias, 0);
        assertAccountState(this.component.setState.calls[0].args[0], {
          aliases: [],
          defaultAlias: null,
        });
      });

      it('removes alias correctly when default alias present and it is not being removed', () => {
        this.account.defaultAlias = this.currentAlias;
        this.account.aliases.push('otheralias');
        this.component = makeComponent({
          account: this.account,
          onAccountUpdated: this.onAccountUpdated,
        });
        spyOn(this.component, '_makeAlias').andCallFake((alias) => alias);
        spyOn(this.component, 'setState');

        this.component._onAccountAliasRemoved('otheralias', 1);
        assertAccountState(this.component.setState.calls[0].args[0], {
          aliases: [this.currentAlias],
          defaultAlias: this.currentAlias,
        });
      });
    });
  });
});
