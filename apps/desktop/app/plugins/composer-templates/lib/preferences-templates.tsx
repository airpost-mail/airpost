import fs from 'fs';
import React from 'react';
import ReactDOM from 'react-dom';
import classNames from 'classnames';
import {
  ComposerEditor,
  ComposerSupport,
  SettingsPage,
  SettingsCard,
  SettingsField,
} from 'mailspring-component-kit';
import { Actions, localized, localizedReactFragment } from 'mailspring-exports';
import { Value } from 'slate';

import TemplateStore, { TemplateItem } from './template-store';
import { parseTemplate, stringifyTemplate } from './template-file';

const {
  Conversion: { convertFromHTML, convertToHTML },
} = ComposerSupport;

interface TemplateEditorProps {
  template: TemplateItem;
  onEditTitle: (title: string) => void;
  onDelete: () => void;
}
class TemplateEditor extends React.Component<
  TemplateEditorProps,
  { editorState: Value; subject: string }
> {
  _composer: ComposerEditor;

  constructor(props) {
    super(props);

    let parsed = { subject: '', body: '' };
    try {
      parsed = parseTemplate(fs.readFileSync(props.template.path).toString());
    } catch (err) {
      // The templates folder can hold a subfolder or an unreadable file; show it empty.
    }
    this.state = { editorState: convertFromHTML(parsed.body), subject: parsed.subject };
  }

  _onSave = () => {
    const outHTML = stringifyTemplate({
      subject: this.state.subject,
      body: convertToHTML(this.state.editorState),
    });
    fs.writeFileSync(this.props.template.path, outHTML);
  };

  _onSubjectChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    this.setState({ subject: e.target.value });
  };

  _onFocusEditor = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.target === ReactDOM.findDOMNode(this._composer)) {
      this._composer.focusEndAbsolute();
    }
  };

  render() {
    const { onEditTitle, onDelete, template } = this.props;
    const { editorState, subject } = this.state;

    return (
      <div className="settings-split-editor template-wrap">
        <SettingsField label={localized('Name')} htmlFor="template-title">
          <input
            type="text"
            id="template-title"
            placeholder={localized('Name')}
            defaultValue={template.name}
            onBlur={(e) => onEditTitle(e.target.value)}
          />
        </SettingsField>
        <SettingsField label={localized('Subject')} htmlFor="template-subject">
          <input
            type="text"
            id="template-subject"
            placeholder={localized('Optional')}
            value={subject}
            onChange={this._onSubjectChange}
            onBlur={this._onSave}
          />
        </SettingsField>
        <div className="template-editor" onClick={this._onFocusEditor}>
          <ComposerEditor
            ref={(c) => (this._composer = c)}
            value={editorState}
            propsForPlugins={{ inTemplateEditor: true }}
            onChange={(change) => this.setState({ editorState: change.value })}
            onBlur={this._onSave}
          />
        </div>
        <div className="template-note">
          {localized(
            'The subject is filled into your draft when you use the template. Leave it blank to keep the subject you already have.'
          )}{' '}
          {localizedReactFragment(
            'Changes are saved automatically. View the %@ for tips and tricks.',
            <a href="https://github.com/ternmail/ternmail#readme">{localized('Templates Guide')}</a>
          )}
        </div>
        <div className="settings-actions">
          <button
            className="btn"
            onClick={() =>
              require('@electron/remote').shell.showItemInFolder(TemplateStore.directory())
            }
          >
            {process.platform === 'darwin'
              ? localized('Show in Finder')
              : localized('Show templates folder')}
          </button>
          <span className="spacer" />
          <button className="btn btn-danger" onClick={onDelete}>
            {localized('Delete')}
          </button>
        </div>
      </div>
    );
  }
}

export default class PreferencesTemplates extends React.Component<
  Record<string, unknown>,
  { selected: TemplateItem; templates: TemplateItem[] }
> {
  static displayName = 'PreferencesTemplates';

  unsubscribers: Array<() => void>;

  constructor(props) {
    super(props);
    this.state = this._getStateFromStores();
  }

  componentDidMount() {
    this.unsubscribers = [
      TemplateStore.listen(() => {
        this.setState(this._getStateFromStores());
      }),
    ];
  }

  componentWillUnmount() {
    this.unsubscribers.forEach((unsubscribe) => unsubscribe());
  }

  _getStateFromStores() {
    const templates = TemplateStore.items();
    const prev = this.state ? this.state.templates : [];
    const lastSelName = this.state && this.state.selected && this.state.selected.name;
    const lastSelIndex = prev.findIndex((t) => t.name === lastSelName);

    // A single name that wasn't there before is a template just made or renamed here.
    const added = prev.length ? templates.filter((t) => !prev.some((p) => p.name === t.name)) : [];
    const selected =
      (added.length === 1 && added[0]) ||
      templates.find((t) => t.name === lastSelName) ||
      templates[lastSelIndex] ||
      templates[0] ||
      null;

    return { templates, selected };
  }

  _onAdd = () => {
    Actions.createTemplate({
      name: localized('Untitled'),
      contents: localized('Insert content here!'),
    });
  };

  _onDelete = () => {
    Actions.deleteTemplate(this.state.selected.name);
  };

  _onEditTitle = (newName: string) => {
    Actions.renameTemplate(this.state.selected.name, newName);
  };

  render() {
    const { selected, templates } = this.state;

    return (
      <SettingsPage
        title={localized('Templates')}
        description={localized('Messages you send often. Insert one from the composer.')}
        className="preferences-templates-container"
      >
        <SettingsCard className="settings-split">
          <div className="settings-split-list template-list" role="listbox">
            {templates.map((t) => (
              <button
                key={t.name}
                role="option"
                aria-selected={t === selected}
                className={classNames('list-item', { selected: t === selected })}
                onClick={() => this.setState({ selected: t })}
              >
                {t.name}
              </button>
            ))}
            <button className="settings-split-new" onClick={this._onAdd}>
              {localized('+ New template')}
            </button>
          </div>
          {selected ? (
            <TemplateEditor
              key={selected.name}
              template={selected}
              onEditTitle={this._onEditTitle}
              onDelete={this._onDelete}
            />
          ) : (
            <div className="settings-split-empty">
              {localized('Make a template to insert it from the composer.')}
            </div>
          )}
        </SettingsCard>
      </SettingsPage>
    );
  }
}
