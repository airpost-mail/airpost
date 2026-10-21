import * as Actions from '../actions';
import BaseStore from './base';
import _ from 'underscore';

export interface IDefaultSignatures {
  // null once the account's default is toggled off.
  [accountId: string]: string | null;
}

// A signature template's fields (templateName, name, title, email, photoURL, ...). The Raw
// HTML choice leaves templateName undefined.
export type SignatureData = { [field: string]: string | undefined };

export interface ISignature {
  id: string;
  title: string;
  body: string;
  data: SignatureData;
}

export interface ISignatureSet {
  [key: string]: ISignature;
}

class _SignatureStore extends BaseStore {
  // activate() sets these from the constructor.
  signatures!: ISignatureSet;
  defaultSignatures!: IDefaultSignatures;
  selectedSignatureId!: string | null;

  unsubscribers?: Array<() => void>;

  constructor() {
    super();
    this.activate(); // for specs
  }

  activate() {
    this.signatures = AppEnv.config.get(`signatures`);
    this.defaultSignatures = AppEnv.config.get(`defaultSignatures`) || {};

    // A clean install starts with no signatures; profiles imported from Mailspring keep theirs.
    if (!this.signatures) {
      this.signatures = {};
    }

    // migrate signatures that didn't have a `data` property
    Object.values(this.signatures).forEach(
      (sig) => (sig.data = sig.data || { title: '', templateName: '' })
    );

    this._autoselectSignatureId();

    if (!this.unsubscribers) {
      this.unsubscribers = [
        Actions.removeSignature.listen(this._onRemoveSignature),
        Actions.upsertSignature.listen(this._onUpsertSignature),
        Actions.selectSignature.listen(this._onSelectSignature),
        Actions.toggleAccount.listen(this._onToggleAccount),
      ];

      AppEnv.config.onDidChange(`signatures`, () => {
        this.signatures = AppEnv.config.get(`signatures`);
        this.trigger();
      });
      AppEnv.config.onDidChange(`defaultSignatures`, () => {
        this.defaultSignatures = AppEnv.config.get(`defaultSignatures`);
        this.trigger();
      });
    }
  }

  deactivate() {
    throw new Error("Unimplemented - core stores shouldn't be deactivated.");
  }

  getSignatures() {
    return this.signatures;
  }

  selectedSignature(): ISignature | undefined {
    const id = this.selectedSignatureId;
    return id === null ? undefined : this.signatures[id];
  }

  getDefaults() {
    return this.defaultSignatures;
  }

  signatureForEmail = (email: string): ISignature | undefined => {
    const id = this.defaultSignatures[email];
    return id === null || id === undefined ? undefined : this.signatures[id];
  };

  _saveSignatures() {
    AppEnv.config.set(`signatures`, this.signatures);
  }

  _saveDefaultSignatures() {
    AppEnv.config.set(`defaultSignatures`, this.defaultSignatures);
  }

  _onSelectSignature = (id: string | null) => {
    this.selectedSignatureId = id;
    this.trigger();
  };

  _autoselectSignatureId() {
    const sigIds = Object.keys(this.signatures);
    this.selectedSignatureId = sigIds.length ? sigIds[0] : null;
  }

  _onRemoveSignature = (signatureToDelete: ISignature) => {
    this.signatures = Object.assign({}, this.signatures);
    delete this.signatures[signatureToDelete.id];
    this._autoselectSignatureId();
    this.trigger();
    this._saveSignatures();
  };

  _onUpsertSignature = (signature: ISignature, id: string) => {
    this.signatures[id] = signature;
    this.trigger();
    this._saveSignatures();
  };

  _onToggleAccount = (email: string) => {
    if (this.defaultSignatures[email] === this.selectedSignatureId) {
      this.defaultSignatures[email] = null;
    } else {
      this.defaultSignatures[email] = this.selectedSignatureId;
    }

    this.trigger();
    this._saveDefaultSignatures();
  };
}

export const SignatureStore = new _SignatureStore();
