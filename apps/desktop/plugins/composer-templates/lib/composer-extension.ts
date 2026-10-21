import { localized, ComposerExtension, Message } from 'mailspring-exports';

export default class TemplatesComposerExtension extends ComposerExtension {
  static warningsForSending({ draft }: { draft: Message }) {
    const warnings: string[] = [];
    if ((draft.body ?? '').search(/<code[^>]*empty[^>]*>/i) > 0) {
      warnings.push(localized('The message contains an empty template area.'));
    }
    return warnings;
  }
}
