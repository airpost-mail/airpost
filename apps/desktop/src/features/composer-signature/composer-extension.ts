import { ComposerExtension } from '../../packages/composer-extension';
import { SignatureStore } from '../../data/stores/signature';
import { applySignature } from './utils';
import { Message } from '../../data/models/message';

export default class SignatureComposerExtension extends ComposerExtension {
  static prepareNewDraft = ({ draft }: { draft: Message }) => {
    if (draft.plaintext) {
      return;
    }

    const from = draft.from && draft.from[0];
    const signatureObj = from ? SignatureStore.signatureForEmail(from.email) : null;
    if (!signatureObj) {
      return;
    }
    draft.body = applySignature(draft.body ?? '', signatureObj);
  };
}
