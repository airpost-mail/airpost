import React from 'react';
import {
  localized,
  AccountStore,
  SignatureStore,
  Actions,
  FocusedPerspectiveStore,
  Utils,
  ISignatureSet,
  ISignature,
  IDefaultSignatures,
  IAliasSet,
  InlineStyleTransformer,
  SanitizeTransformer,
} from 'mailspring-exports';
import classNames from 'classnames';
import { SettingsPage, SettingsCard, SettingsField } from 'mailspring-component-kit';
import { Button } from '../../components/kit/button';

import { ResolveSignatureData, RenderSignatureData, DataShape } from './constants';
import SignatureAccountDefaultPicker from './account-default-picker';
import SignatureTemplatePicker from './template-picker';
import SignaturePhotoPicker from './photo-picker';
import Templates from './templates';

interface SignatureEditorProps {
  signature: ISignature;
  defaults: IDefaultSignatures;
  accountsAndAliases: IAliasSet;
}

class SignatureEditor extends React.Component<SignatureEditorProps> {
  _onTitleChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    const sig = this.props.signature;
    Actions.upsertSignature({ ...sig, title: event.target.value }, sig.id);
  };

  _onRawBodyChange = (event: React.ChangeEvent<HTMLTextAreaElement>) => {
    this._saveRawBody(event.target.value);
  };

  // The raw source field is a plain <textarea>, so pasting rich content would normally
  // insert only the text/plain flavor of the clipboard. When the clipboard also carries
  // HTML (eg: copying a signature out of a webmail client), insert the markup itself so
  // the user gets the formatting they copied rather than a wall of unstyled text.
  _onRawBodyPaste = (event: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const plain = event.clipboardData.getData('text/plain');

    // If the plain text flavor already looks like markup, the user is copying HTML
    // source (from an editor, view-source, another signature) and expects it verbatim.
    if (/<[a-z!/][\s\S]*>/i.test(plain)) {
      return;
    }

    // Sanitize with the same rules we apply to pasted content in the composer — this
    // strips scripts, event handlers and unsafe URI schemes before the markup can be
    // saved and rendered into the preview / composer.
    const html = SanitizeTransformer.runSync(event.clipboardData.getData('text/html')).trim();
    if (!html) {
      return;
    }

    event.preventDefault();

    const el = event.currentTarget;

    // Prefer execCommand so the paste joins the textarea's native undo stack. It emits
    // an `input` event, which React surfaces as our onChange handler.
    if (document.execCommand('insertText', false, html)) {
      return;
    }

    const { selectionStart, selectionEnd, value } = el;
    el.value = `${value.slice(0, selectionStart)}${html}${value.slice(selectionEnd)}`;
    el.selectionStart = el.selectionEnd = selectionStart + html.length;
    this._saveRawBody(el.value);
  };

  _saveRawBody = async (value: string) => {
    const sig = this.props.signature;
    let body = value;
    try {
      body = await InlineStyleTransformer.run(body);
    } catch (err) {
      //no-op
    }
    Actions.upsertSignature({ ...sig, body }, sig.id);
  };

  _onDataFieldChange = (event: { target: { id: string; value: string } }) => {
    const { id, value } = event.target;
    const sig = this.props.signature;

    // If you have raw selected and are switching back to a template,
    // display a warning UNLESS the html is an unmodified template HTML
    if (id === 'templateName' && !sig.data.templateName && value) {
      const htmlMatchesATemplate = Templates.find(
        (t) => sig.body === RenderSignatureData({ ...sig.data, templateName: t.name })
      );
      if (!htmlMatchesATemplate) {
        const idx = require('@electron/remote').dialog.showMessageBoxSync({
          type: 'warning',
          buttons: [localized('Cancel'), localized('Continue')],
          message: localized('Revert custom HTML?'),
          detail: localized(
            "Switching back to a signature template will overwrite the custom HTML you've entered."
          ),
        });
        if (idx === 0) {
          return;
        }
      }
    }

    // apply change
    sig.data = { ...sig.data, [id]: value };

    // re-render
    if (sig.data.templateName) {
      const template = Templates.find((t) => t.name === sig.data.templateName);
      if (template) {
        sig.body = RenderSignatureData(sig.data);
      }
    }

    Actions.upsertSignature(sig, sig.id);
  };

  render() {
    const { accountsAndAliases, defaults, signature } = this.props;
    const data = signature.data || {};
    const resolvedData = ResolveSignatureData(data);

    return (
      <div className="settings-split-editor signature-wrap">
        <SettingsField label={localized('Name')} htmlFor="signature-title">
          <input
            type="text"
            id="signature-title"
            placeholder={localized('Name')}
            value={signature.title || ''}
            onChange={this._onTitleChange}
          />
        </SettingsField>
        <SettingsField label={localized('Default for')}>
          <SignatureAccountDefaultPicker
            signature={signature}
            accountsAndAliases={accountsAndAliases}
            defaults={defaults}
          />
        </SettingsField>

        {/* Sanitized like a message body: signatures can come from other clients through
        the engine's settings, and this window runs with Node. */}
        <div
          className="signature-preview"
          dangerouslySetInnerHTML={{ __html: SanitizeTransformer.runSync(signature.body || '') }}
        />

        <div className="signature-subhead">{localized('Style')}</div>
        <SignatureTemplatePicker resolvedData={resolvedData} onChange={this._onDataFieldChange} />

        {!resolvedData.templateName ? (
          <>
            <label htmlFor="signature-body" className="signature-subhead">
              {localized('HTML')}
            </label>
            <textarea
              id="signature-body"
              key={`textarea ${signature.id}`}
              className="raw-html"
              spellCheck={false}
              onChange={this._onRawBodyChange}
              onPaste={this._onRawBodyPaste}
              defaultValue={signature.body || ''}
            />
            <div className="signature-note">
              {localized(
                'Moving from another mail app? Select your existing signature in an email and copy-paste it into the box above.'
              )}
            </div>
          </>
        ) : (
          <>
            <div className="signature-subhead">{localized('Details')}</div>
            <div className="signature-information">
              {DataShape.map((item) => (
                <div className="field" key={item.key}>
                  <label htmlFor={item.key}>{item.label}</label>
                  <input
                    type="text"
                    onChange={this._onDataFieldChange}
                    placeholder={item.placeholder}
                    id={item.key}
                    value={data[item.key] || ''}
                  />
                </div>
              ))}
              <SignaturePhotoPicker
                data={data}
                resolvedURL={resolvedData.photoURL}
                onChange={this._onDataFieldChange}
              />
            </div>
          </>
        )}

        <div className="settings-actions">
          <span className="spacer" />
          <Button variant="destructive" onClick={() => Actions.removeSignature(signature)}>
            {localized('Delete')}
          </Button>
        </div>
      </div>
    );
  }
}

interface PreferencesSignaturesState {
  signatures: ISignatureSet;
  selectedSignature: ISignature;
  defaults: IDefaultSignatures;
  accountsAndAliases: IAliasSet;
}

export default class PreferencesSignatures extends React.Component<
  Record<string, unknown>,
  PreferencesSignaturesState
> {
  static displayName = 'PreferencesSignatures';

  unsubscribers = [];

  constructor(props) {
    super(props);
    this.state = this._getStateFromStores();
  }

  componentDidMount() {
    this.unsubscribers = [SignatureStore.listen(this._onChange)];
  }

  componentWillUnmount() {
    this.unsubscribers.forEach((unsubscribe) => unsubscribe());
  }

  _onChange = () => {
    this.setState(this._getStateFromStores());
  };

  _getStateFromStores() {
    return {
      signatures: SignatureStore.getSignatures(),
      selectedSignature: SignatureStore.selectedSignature(),
      defaults: SignatureStore.getDefaults(),
      accountsAndAliases: AccountStore.aliases(),
    };
  }

  _onAddSignature = () => {
    const activeIds = FocusedPerspectiveStore.current().accountIds || AccountStore.accountIds();
    const activeAccount = AccountStore.accountForId(activeIds[0]);
    const id = Utils.generateTempId();

    let data = {};
    let body = null;
    if (this.state.selectedSignature) {
      data = { ...this.state.selectedSignature.data };
      body = this.state.selectedSignature.body;
    } else {
      data = {
        templateName: Templates[0].name,
        name: activeAccount.name,
        email: activeAccount.emailAddress,
      };
      body = RenderSignatureData(data);
    }

    Actions.upsertSignature({ id, title: localized('Untitled'), body, data }, id);
    Actions.selectSignature(id);
  };

  render() {
    const { signatures, selectedSignature } = this.state;
    const isSelected = (sig: ISignature) => !!selectedSignature && sig.id === selectedSignature.id;
    return (
      <SettingsPage
        title={localized('Signatures')}
        description={localized('Added to the end of your messages.')}
        className="preferences-signatures-container"
      >
        <SettingsCard className="settings-split">
          <div className="settings-split-list signature-list" role="listbox">
            {Object.values(signatures).map((sig) => (
              // eslint-disable-next-line raw-controls -- a listbox option, not a kit Button
              <button
                key={sig.id}
                role="option"
                aria-selected={isSelected(sig)}
                className={classNames('list-item', { selected: isSelected(sig) })}
                onClick={() => Actions.selectSignature(sig.id)}
              >
                {sig.title || localized('Untitled')}
              </button>
            ))}
            <Button variant="link" className="settings-split-new" onClick={this._onAddSignature}>
              {localized('+ New signature')}
            </Button>
          </div>
          {selectedSignature ? (
            <SignatureEditor
              signature={selectedSignature}
              defaults={this.state.defaults}
              accountsAndAliases={this.state.accountsAndAliases}
            />
          ) : (
            <div className="settings-split-empty">
              {localized('Make a signature to add it to the end of your messages.')}
            </div>
          )}
        </SettingsCard>
      </SettingsPage>
    );
  }
}
