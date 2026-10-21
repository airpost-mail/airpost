import BaseStore from '../../data/stores/base';
import fs from 'fs';
import path from 'path';
import * as Utils from '../../data/models/utils';
import { Message } from '../../data/models/message';
import MessageBodyProcessor from '../../mail/body/processor';
import CategoryStore from '../../data/stores/category';
import * as AutoloadImagesActions from './actions';

class AutoloadImagesStore extends BaseStore {
  _whitelistEmails = {};
  _whitelistMessageIds = {};
  _whitelistEmailsPath = path.join(AppEnv.getConfigDirPath(), 'autoload-images-whitelist.txt');

  constructor() {
    super();

    this._loadWhitelist();

    this.listenTo(AutoloadImagesActions.temporarilyEnableImages, this._onTemporarilyEnableImages);
    this.listenTo(AutoloadImagesActions.permanentlyEnableImages, this._onPermanentlyEnableImages);

    AppEnv.config.onDidChange('core.reading.autoloadImages', () => {
      MessageBodyProcessor.resetCache();
    });
  }

  // The regexes only decide whether to show the "images blocked" banner. Blocking itself is
  // the message document's CSP (src/mail/content-security-policy.ts), and "Show Images" lifts it
  // only for http(s) URLs, so that is all these count (cid: and data: images load either
  // way). Matches an http(s) URL right after
  // - src= / background= / srcset= / poster= ... (quoted or not)
  // - url( or image-set( anywhere in CSS, quoted or not
  // - @import "..." / @import '...'
  getImagesRegexp = () => {
    return /(?:\b(?:src|background|placeholder|icon|poster|srcset)\s*=\s*['"]?\s*|\b(?:url|image-set)\(\s*['"]?\s*|@import\s+['"]\s*)https?:/i;
  };

  getLinkTagRegexp = () => {
    // Use \s+ rather than a literal space so tabs/newlines between <link and
    // its attributes don't slip a tracking <link> through unmatched.
    return /<link\s+[^>]+>/gi;
  };

  remoteImagesAllowed = (message: Message) => {
    const spam = CategoryStore.getSpamCategory(message.accountId);
    const inSpam = !!spam && message.folderIds().includes(spam.id);

    if (AppEnv.config.get('core.reading.autoloadImages') && !inSpam) {
      return true;
    }
    if (this._whitelistEmails[Utils.toEquivalentEmailForm(message.fromContact().email)]) {
      return true;
    }
    return !!this._whitelistMessageIds[message.id];
  };

  shouldBlockImagesIn = (message: Message) => {
    if (this.remoteImagesAllowed(message)) {
      return false;
    }
    const body = message.body || '';
    return this.getImagesRegexp().test(body) || this.getLinkTagRegexp().test(body);
  };

  _loadWhitelist = () => {
    fs.exists(this._whitelistEmailsPath, (exists) => {
      if (!exists) {
        return;
      }

      fs.readFile(this._whitelistEmailsPath, (err, body) => {
        if (err || !body) {
          console.log(err);
          return;
        }

        this._whitelistEmails = {};
        body
          .toString()
          .split(/[\n\r]+/)
          .forEach((email) => {
            this._whitelistEmails[Utils.toEquivalentEmailForm(email)] = true;
          });
        // Bodies processed before the list loaded were given the blocking policy.
        MessageBodyProcessor.resetCache();
        this.trigger();
      });
    });
  };

  _saveWhitelist = () => {
    const data = Object.keys(this._whitelistEmails).join('\n');
    fs.writeFile(this._whitelistEmailsPath, data, (err) => {
      if (err) {
        console.error(`AutoloadImagesStore could not save whitelist: ${err.toString()}`);
      }
    });
  };

  _onTemporarilyEnableImages = (message: Message) => {
    this._whitelistMessageIds[message.id] = true;
    MessageBodyProcessor.resetCache();
    this.trigger();
  };

  _onPermanentlyEnableImages = (message: Message) => {
    const email = Utils.toEquivalentEmailForm(message.fromContact().email);
    this._whitelistEmails[email] = true;
    MessageBodyProcessor.resetCache();
    setTimeout(this._saveWhitelist, 1);
    this.trigger();
  };
}

export default new AutoloadImagesStore();
