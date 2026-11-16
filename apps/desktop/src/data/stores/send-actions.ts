import _str from 'underscore.string';
import BaseStore from './base';
import * as Actions from '../actions';
import { Message } from '../models/message';
import { SendDraftTask } from '../tasks/send-draft';
import * as ExtensionRegistry from '../../packages/extension-registry';

export interface ISendAction {
  title: string;
  iconUrl: string | null;
  configKey: string;
  isAvailableForDraft: ({ draft }: { draft: Message }) => boolean;
  performSendAction: ({ draft }: { draft: Message }) => void;
}

const ACTION_CONFIG_KEY = 'core.sending.defaultSendType';
const DefaultSendActionKey = 'send';
// The send actions an engine performs when it sends a scheduled draft itself (airpost-sync
// expiration_executor SEND_ACTIONS).
const ENGINE_SEND_ACTION_KEYS = [DefaultSendActionKey, 'send-and-archive'];
const DefaultSendAction: ISendAction = {
  title: 'Send',
  iconUrl: null,
  configKey: DefaultSendActionKey,
  isAvailableForDraft: () => true,
  performSendAction: ({ draft }) => Actions.queueTask(SendDraftTask.forSending(draft)),
};

function verifySendAction(sendAction: ISendAction, extension: { name?: string } = {}) {
  const { name } = extension;
  if (typeof sendAction.title !== 'string') {
    throw new Error(`${name}.sendActions must return objects containing a string "title"`);
  }
  if (!(sendAction.performSendAction instanceof Function)) {
    throw new Error(
      `${name}.sendActions must return objects containing an "performSendAction" function that will be called when the action is selected`
    );
  }
  return true;
}

class _SendActionsStore extends BaseStore {
  _sendActions: ISendAction[] = [];
  _unsubscribers: (() => void)[] = [];

  constructor() {
    super();
    this._unsubscribers = [ExtensionRegistry.Composer.listen(this._onComposerExtensionsChanged)];
    this._onComposerExtensionsChanged();
  }

  get DefaultSendActionKey() {
    return DefaultSendActionKey;
  }

  get DefaultSendAction() {
    return DefaultSendAction;
  }

  getSendActions() {
    return this._sendActions;
  }

  collectSendActions() {
    const all = [DefaultSendAction];
    for (const ext of ExtensionRegistry.Composer.extensions()) {
      const extActions = (ext.sendActions && ext.sendActions()) || [];
      for (const extAction of extActions) {
        try {
          verifySendAction(extAction, ext);
          extAction.configKey = _str.dasherize(extAction.title.toLowerCase());
          all.push(extAction);
        } catch (err) {
          AppEnv.reportError(err);
        }
      }
    }
    return all;
  }

  sendActionForKey(configKey: string) {
    return this._sendActions.find((a) => a.configKey === configKey);
  }

  // Whether an engine with the expiration executor performs this after-send action itself.
  enginePerformsSendAction(actionKey: string = DefaultSendActionKey) {
    return ENGINE_SEND_ACTION_KEYS.includes(actionKey);
  }

  // Whether a scheduled send of `draft` with `actionKey` must be finalized by a window:
  // an action the engine doesn't perform, or a composer
  // extension that transforms the body for sending, which only a window can run. Such a
  // send-later row is marked `clientFinalize` and the engine leaves it to the client. Open and
  // link tracking are the engine's own transforms (T22).
  needsClientFinalize(draft: Message, actionKey: string = DefaultSendActionKey) {
    if (!this.enginePerformsSendAction(actionKey)) return true;
    // Every ComposerExtension subclass inherits the base class's no-op.
    return ExtensionRegistry.Composer.extensions().some((ext) =>
      Object.prototype.hasOwnProperty.call(ext, 'applyTransformsForSending')
    );
  }

  orderedSendActionsForDraft(draft: Message) {
    const configKeys = this._sendActions.map(({ configKey }) => configKey);

    let preferredKey = AppEnv.config.get(ACTION_CONFIG_KEY);
    if (!preferredKey || !configKeys.includes(preferredKey)) {
      preferredKey = DefaultSendActionKey;
    }

    let preferred = this._sendActions.find((a) => a.configKey === preferredKey);
    if (!preferred || !preferred.isAvailableForDraft({ draft })) {
      preferred = DefaultSendAction;
    }
    const rest = this._sendActions.filter(
      (action) => action !== preferred && action.isAvailableForDraft({ draft })
    );

    return [preferred, ...rest];
  }

  _onComposerExtensionsChanged = () => {
    this._sendActions = this.collectSendActions();
    this.trigger();
  };
}

export const SendActionsStore = new _SendActionsStore();
