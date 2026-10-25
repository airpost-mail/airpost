/* eslint global-require: 0*/
// The templates are the engine's (`templates.*`, files in `<config>/templates` that the CLI,
// MCP and web read too). This store lists them for the picker and the preferences tab and
// inserts one into an open draft; the engine reads, writes, names and parses the files.

import path from 'path';
import fs from 'fs';
import { localized, DraftStore, Actions, Engine } from 'mailspring-exports';
import BaseStore from 'mailspring-store';

import { Dialogs } from '../../../src/platform/app-api';
import EngineEvents from '../../../src/engine/client/events';
import { Method, TemplateInfo } from '../../../src/engine/protocol';

// Support accented characters in template names
// https://regex101.com/r/nD3eY8/1
const INVALID_TEMPLATE_NAME_REGEX = /[^a-zA-Z\u00C0-\u017F0-9_\- ]+/g;

// Set once the welcome template has been added, or there were templates already.
const WELCOMED_KEY = 'composer-templates.welcomed';

export interface TemplateItem {
  // The file name.
  id: string;
  name: string;
  subject: string;
}

export interface TemplateContents {
  subject: string;
  body: string;
}

// The engine's English error text without its "invalid params: " prefix, in the app's language
// when it is one of the desktop's strings.
function engineErrorText(err: unknown) {
  const message = String((err as Error)?.message || err).replace(/^invalid params: /, '');
  return localized(message);
}

class TemplateStore extends BaseStore {
  private _items: TemplateItem[] = [];
  private _dialogOpen = false;
  private _templatesDir = path.join(AppEnv.getConfigDirPath(), 'templates');

  constructor() {
    super();

    this.listenTo(Actions.insertTemplateId, this._onInsertTemplateId);
    this.listenTo(Actions.createTemplate, this._onCreateTemplate);
    this.listenTo(Actions.showTemplates, this._onShowTemplates);
    this.listenTo(Actions.deleteTemplate, this._onDeleteTemplate);
    this.listenTo(Actions.renameTemplate, this._onRenameTemplate);

    EngineEvents.whenConnected().then(() => this._populate({ welcome: true, quiet: true }));
  }

  // For "Show in Finder".
  directory() {
    return this._templatesDir;
  }

  items() {
    return this._items;
  }

  // Lists the templates again. The picker and the preferences tab call it when they open:
  // another client may have changed them.
  refresh() {
    return this._populate();
  }

  // `quiet`: a failure is logged, not shown (the list read when the window connects).
  async _populate({
    welcome = false,
    quiet = false,
  }: { welcome?: boolean; quiet?: boolean } = {}): Promise<void> {
    let templates: TemplateInfo[];
    try {
      ({ templates } = await Engine.request<{ templates: TemplateInfo[] }>(Method.TemplatesList));
    } catch (err) {
      if (quiet) {
        console.warn(`composer-templates: could not list the templates: ${err}`);
        return;
      }
      AppEnv.showErrorDialog({
        title: localized('Cannot scan templates directory'),
        message: localized(
          'Tern was unable to read the contents of your templates directory (%@). You may want to delete this folder or ensure filesystem permissions are set correctly.',
          this._templatesDir
        ),
      });
      return;
    }
    if (welcome && !AppEnv.config.get(WELCOMED_KEY)) {
      AppEnv.config.set(WELCOMED_KEY, true);
      if (templates.length === 0 && (await this._addWelcomeTemplate())) {
        return this._populate();
      }
    }
    this._items = templates.map(({ id, name, subject }) => ({ id, name, subject }));
    this.trigger(this);
  }

  // The body and subject of a template.
  async contents(template: TemplateItem): Promise<TemplateContents> {
    const { template: t } = await Engine.request<{ template: TemplateInfo }>(Method.TemplatesGet, {
      name: template.id,
    });
    return { subject: t.subject, body: t.body || '' };
  }

  // Writes over the template's file.
  async update(template: TemplateItem, { subject, body }: TemplateContents) {
    try {
      await Engine.request(Method.TemplatesSave, {
        name: template.name,
        subject,
        body,
        replace: true,
      });
      template.subject = subject.trim();
    } catch (err) {
      this._displayError(engineErrorText(err));
    }
  }

  _onCreateTemplate({
    headerMessageId,
    name,
    contents,
    subject,
  }: {
    headerMessageId?: string;
    name?: string;
    contents?: string;
    subject?: string;
  } = {}) {
    if (headerMessageId) {
      this._onCreateTemplateFromDraft(headerMessageId);
      return;
    }
    if (!name || name.length === 0) {
      this._displayError(localized('You must provide a name for your template.'));
      return;
    }
    if (!contents || contents.length === 0) {
      this._displayError(localized('You must provide contents for your template.'));
      return;
    }
    this.saveNewTemplate(name, { subject: subject || '', body: contents });
  }

  // The engine makes the template from the saved draft: the text above its signature and
  // quote, named after its subject, with the subject unless it answers a message.
  async _onCreateTemplateFromDraft(headerMessageId: string) {
    const session = await DraftStore.sessionForClientId(headerMessageId);
    await session.changes.commit();
    const draft = session.draft();
    if (!draft) throw new Error('TemplateStore: the draft session has no draft');
    try {
      await Engine.request(Method.TemplatesSave, {
        accountId: draft.accountId,
        messageId: draft.id,
      });
    } catch (err) {
      this._displayError(engineErrorText(err));
      return;
    }
    await this._populate();
    this._onShowTemplates();
  }

  _onShowTemplates() {
    Actions.switchPreferencesTab('Templates');
    Actions.openPreferences();
  }

  _displayError(message: string) {
    AppEnv.showErrorDialog({ title: localized('Template Creation Error'), message });
  }

  /**
   * Resolves true when the first button is clicked. The dialog doesn't block the window, so
   * while one is open a second request resolves false without showing another.
   */
  async _displayDialog(title: string, message: string, buttons: string[]): Promise<boolean> {
    if (this._dialogOpen) return false;
    this._dialogOpen = true;
    try {
      const response = await Dialogs.ask(
        {
          title: title,
          message: title,
          detail: message,
          buttons: buttons,
          type: 'info',
        },
        'none'
      );
      return response === 0;
    } finally {
      this._dialogOpen = false;
    }
  }

  // The engine checks names too; this gives the message in the app's language first.
  _nameError(name: string) {
    if (!name || name.length === 0) {
      return localized('You must provide a template name.');
    }
    if (name.match(INVALID_TEMPLATE_NAME_REGEX)) {
      return localized(
        'Invalid template name! Names can only contain letters, numbers, spaces, dashes, and underscores.'
      );
    }
    return null;
  }

  // Saves under `name`, or `name 1`, `name 2`, … when it is taken, and shows the templates.
  async saveNewTemplate(name: string, { subject, body }: TemplateContents) {
    const error = this._nameError(name);
    if (error) {
      this._displayError(error);
      return;
    }
    try {
      await Engine.request(Method.TemplatesSave, { name, subject, body });
    } catch (err) {
      this._displayError(engineErrorText(err));
      return;
    }
    await this._populate();
    this._onShowTemplates();
  }

  async _onDeleteTemplate(name: string) {
    const template = this._items.find((t) => t.name === name);
    if (!template) {
      return;
    }

    if (
      await this._displayDialog(
        localized('Delete Template?'),
        localized('The template and its file will be permanently deleted.'),
        [localized('Delete'), localized('Cancel')]
      )
    ) {
      try {
        await Engine.request(Method.TemplatesDelete, { name: template.id });
      } catch (err) {
        AppEnv.showErrorDialog(engineErrorText(err));
      }
      await this._populate();
    }
  }

  async _onRenameTemplate(name: string, newName: string) {
    const template = this._items.find((t) => t.name === name);
    if (!template || newName === name) {
      return;
    }
    const error = this._nameError(newName);
    if (error) {
      this._displayError(error);
      return;
    }
    try {
      const contents = await this.contents(template);
      await Engine.request(Method.TemplatesSave, { name: newName, ...contents, replace: true });
      await Engine.request(Method.TemplatesDelete, { name: template.id });
    } catch (err) {
      this._displayError(engineErrorText(err));
    }
    await this._populate();
  }

  async _onInsertTemplateId({
    templateId,
    headerMessageId,
  }: { templateId?: string; headerMessageId?: string } = {}) {
    const template = this._items.find((t) => t.id === templateId);
    if (!template) throw new Error(`TemplateStore: no template with id ${templateId}`);
    const { subject: templateSubject, body: templateBody } = await this.contents(template);
    const session = await DraftStore.sessionForClientId(headerMessageId);
    const draft = session.draft();
    if (!draft) throw new Error('TemplateStore: the draft session has no draft');

    // Never touch the subject of a reply or a forward - the "Re:" / "Fwd:" prefix
    // and the original subject tie the message to the thread it came from. (The
    // composer doesn't even show the subject field when you're replying.)
    const canApplySubject =
      !!templateSubject && !draft.replyToHeaderMessageId && !draft.forwardedHeaderMessageId;
    const draftSubject = (draft.subject || '').trim();

    const bodyWouldBeReplaced = !draft.pristine && !draft.hasEmptyBody();
    const subjectWouldBeReplaced =
      canApplySubject && draftSubject.length > 0 && draftSubject !== templateSubject;

    let proceed = true;
    if (bodyWouldBeReplaced || subjectWouldBeReplaced) {
      proceed = await this._displayDialog(
        localized('Replace draft contents?'),
        localized(
          'It looks like your draft already has some content. Loading this template will overwrite all draft contents.'
        ),
        [localized('Replace contents'), localized('Cancel')]
      );
    }

    if (proceed) {
      // Read the body again: the draft may have changed while the dialog was open.
      const current = session.draft()?.body ?? draft.body ?? '';
      let insertion = current.length;
      for (const s of [
        '<signature',
        '<div class="gmail_quote_attribution"',
        '<blockquote class="gmail_quote"',
      ]) {
        const i = current.indexOf(s);
        if (i !== -1) {
          insertion = Math.min(insertion, i);
        }
      }
      const changes: { body: string; subject?: string } = {
        body: `${templateBody}${current.substr(insertion)}`,
      };
      if (canApplySubject) {
        changes.subject = templateSubject;
      }
      session.changes.add(changes);
    }
  }

  // The welcome template in the app's language, from the plugin's assets, whose first line is
  // the template file's subject line.
  async _addWelcomeTemplate() {
    const asset = (name: string) => path.join(__dirname, '..', 'assets', `${name}.html`);
    let name = localized('Welcome to Templates');
    if (!fs.existsSync(asset(name))) name = 'Welcome to Templates';
    try {
      const file = await fs.promises.readFile(asset(name), 'utf8');
      const meta = /^<meta name="subject" content="([^"]*)"\/>\n/.exec(file);
      await Engine.request(Method.TemplatesSave, {
        name,
        subject: meta ? meta[1] : '',
        body: meta ? file.slice(meta[0].length) : file,
      });
      return true;
    } catch (err) {
      console.warn('composer-templates: could not add the welcome template', err);
      return false;
    }
  }
}

export default new TemplateStore();
