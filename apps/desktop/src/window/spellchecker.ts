import fs from 'fs';
import path from 'path';
import { App, Spellchecker as SessionSpellchecker } from '../platform/app-api';

const customDictFilePath = path.join(AppEnv.getConfigDirPath(), 'custom-dict.json');

// The window session's spellchecker lives in main; this sets its language and dictionary.
class Spellchecker {
  constructor() {
    // Nobody will notice if spellcheck isn't available for a few seconds and it
    // takes a considerable amount of time to startup (212ms in dev mode on my 2017 MBP)
    const initHandler = () => {
      this._migrateFromCustomDict();

      const initialLanguage = AppEnv.config.get('core.composing.spellcheckDefaultLanguage');
      this._switchToLanguage(initialLanguage);

      // Monitor language setting for changes. Note that as the user types in a draft we
      // change spellcheck to that language.
      AppEnv.config.onDidChange('core.composing.spellcheckDefaultLanguage', (value) => {
        this._switchToLanguage(value.newValue);
      });
    };

    if (AppEnv.inSpecMode()) {
      initHandler();
    } else {
      setTimeout(initHandler, 5000);
    }
  }

  _switchToLanguage = async (requested: string | undefined | null) => {
    const lang: string = requested ? requested : App.facts().locale || 'en-US';
    const supported = await SessionSpellchecker.availableLanguages();
    if (supported.includes(lang)) {
      await SessionSpellchecker.setLanguages([lang]);
    } else if (lang.includes('-') && supported.includes(lang.split('-')[0])) {
      await SessionSpellchecker.setLanguages([lang.split('-')[0]]);
    }
  };

  _migrateFromCustomDict = () => {
    fs.readFile(customDictFilePath, (err, data) => {
      let fileData: any = data;
      if (err) {
        if (err.code === 'ENOENT') {
          // File doesn't exist, we haven't saved any words yet
          fileData = '{}';
        } else {
          AppEnv.reportError(err);
          return;
        }
      }
      const loadedDict = JSON.parse(fileData);
      Object.keys(loadedDict).forEach((word) => SessionSpellchecker.addWord(word));
      fs.unlink(customDictFilePath, () => {});
    });
  };

  learnWord = (word: string) => {
    // TODO: Ensure that this work. Somehow on my Ubuntu 20.12., this does not add words to the custom dict.
    SessionSpellchecker.addWord(word);
  };

  unlearnWord = (word: string) => {
    SessionSpellchecker.removeWord(word);
  };
}

export default new Spellchecker();
