import { Utils, KeyManager } from 'mailspring-exports';
import { IdentityStore } from '../../src/flux/stores/identity-store';
import * as MailspringAPIRequest from '../../src/flux/mailspring-api-request';

const TEST_NYLAS_ID = 'icihsnqh4pwujyqihlrj70vh';

describe('IdentityStore', function identityStoreSpec() {
  beforeEach(() => {
    this.identityJSON = {
      firstName: 'Mailspring 050',
      lastName: 'Test',
      email: 'mailspring050test@evanmorikawa.com',
      id: TEST_NYLAS_ID,
      token: 'secret token',
    };
  });

  describe('saveIdentity', () => {
    beforeEach(() => {
      IdentityStore._identity = this.identityJSON;
      spyOn(KeyManager, 'deletePassword');
      spyOn(KeyManager, 'replacePassword');
      spyOn(IdentityStore, 'trigger');
      spyOn(AppEnv.config, 'set');
      spyOn(AppEnv.config, 'unset');
    });

    it('clears passwords if unsetting', async () => {
      await IdentityStore.saveIdentity(null);
      expect(KeyManager.deletePassword).toHaveBeenCalled();
      expect(KeyManager.replacePassword).not.toHaveBeenCalled();
      expect(AppEnv.config.set).toHaveBeenCalled();
      const ident = (AppEnv.config.set as jasmine.Spy).calls[0].args[1];
      expect(ident).toBe(null);
    });

    it('applies changes synchronously', async () => {
      const firstName = () => IdentityStore.identity().firstName;
      expect(firstName()).toBe('Mailspring 050');

      const next = JSON.parse(JSON.stringify(this.identityJSON));
      next.firstName = 'Renamed';
      await IdentityStore.saveIdentity(next);
      expect(firstName()).toBe('Renamed');
    });
  });

  describe('returning the identity object', () => {
    beforeEach(() => {
      spyOn(IdentityStore, 'saveIdentity').andReturn(Promise.resolve());
    });
    it('returns the identity as null if it looks blank', () => {
      IdentityStore._identity = null;
      expect(IdentityStore.identity()).toBe(null);
      IdentityStore._identity = {} as any;
      expect(IdentityStore.identity()).toBe(null);
      IdentityStore._identity = { token: 'bad' } as any;
      expect(IdentityStore.identity()).toBe(null);
    });

    it('returns a proper clone of the identity', () => {
      IdentityStore._identity = { id: 'bar', deep: { obj: 'baz' } } as any;
      const ident = IdentityStore.identity() as any;
      (IdentityStore._identity as any).deep.obj = 'changed';
      expect(ident.deep.obj).toBe('baz');
    });
  });

  describe('fetchIdentity', () => {
    beforeEach(() => {
      IdentityStore._identity = this.identityJSON;
      spyOn(IdentityStore, 'saveIdentity');
      spyOn(AppEnv, 'reportError');
      spyOn(console, 'error');
    });

    it('saves the identity returned', async () => {
      const resp = Utils.deepClone(this.identityJSON);
      resp.lastName = 'Updated';
      spyOn(MailspringAPIRequest, 'makeRequest').andCallFake(() => {
        return Promise.resolve(resp);
      });
      await IdentityStore.fetchIdentity();
      expect(MailspringAPIRequest.makeRequest).toHaveBeenCalled();
      const options = (MailspringAPIRequest.makeRequest as jasmine.Spy).calls[0].args[0];
      expect(options.path).toEqual('/api/me');
      expect(IdentityStore.saveIdentity).toHaveBeenCalled();
      const newIdent = (IdentityStore.saveIdentity as jasmine.Spy).calls[0].args[0];
      expect(newIdent.lastName).toBe('Updated');
      expect(AppEnv.reportError).not.toHaveBeenCalled();
    });

    it('errors if the json is invalid', async () => {
      spyOn(MailspringAPIRequest, 'makeRequest').andCallFake(() => {
        return Promise.resolve({});
      });
      await IdentityStore.fetchIdentity();
      expect(AppEnv.reportError).toHaveBeenCalled();
      expect(IdentityStore.saveIdentity).not.toHaveBeenCalled();
    });

    it('resolves with the current identity and does not report an error if the request rejects', async () => {
      spyOn(MailspringAPIRequest, 'makeRequest').andCallFake(() => {
        return Promise.reject(new Error('Failed to fetch'));
      });
      const result = await IdentityStore.fetchIdentity();
      expect(result).toEqual(this.identityJSON);
      expect(AppEnv.reportError).not.toHaveBeenCalled();
      expect(IdentityStore.saveIdentity).not.toHaveBeenCalled();
    });
  });
});
