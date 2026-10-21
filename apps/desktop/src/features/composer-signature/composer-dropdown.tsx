import React from 'react';
import { localized } from '../../intl/localize';
import * as Actions from '../../data/actions';
import { SignatureStore, ISignature, ISignatureSet } from '../../data/stores/signature';
import { DraftEditingSession, MessageWithEditorState } from '../../data/drafts/editing-session';
import { Account } from '../../data/models/account';
import { Menu } from '../../components/menu';
import { RetinaImg } from '../../components/retina-img';
import { ButtonDropdown } from '../../components/button-dropdown';

import { applySignature, currentSignatureIdSlate, currentSignatureId } from './utils';

const MenuItem = Menu.Item;

export default class SignatureComposerDropdown extends React.Component<
  {
    draft: MessageWithEditorState;
    draftFromEmail?: string;
    session: DraftEditingSession;
    accounts: Account[];
  },
  { signatures: ISignatureSet }
> {
  static displayName = 'SignatureComposerDropdown';

  static containerRequired = false;

  _staticIcon = (
    <RetinaImg
      className="signature-button"
      name="top-signature-dropdown.png"
      mode={RetinaImg.Mode.ContentIsMask}
    />
  );

  _staticFooterItems = [
    <div
      key="edit"
      className="item item-edit"
      onMouseDown={() => {
        Actions.switchPreferencesTab('Signatures');
        Actions.openPreferences();
      }}
    >
      <span>{localized('Edit Signatures...')}</span>
    </div>,
  ];

  unsubscribers: Array<() => void> = [];

  constructor(props: SignatureComposerDropdown['props']) {
    super(props);
    this.state = this._getStateFromStores();
  }

  componentDidMount = () => {
    this.unsubscribers = [
      SignatureStore.listen(() => {
        this.setState(this._getStateFromStores());
      }),
    ];
  };

  componentDidUpdate(previousProps: { draftFromEmail?: string }) {
    const email = this.props.draftFromEmail;
    if (previousProps.draftFromEmail !== email) {
      const nextDefaultSignature = email ? SignatureStore.signatureForEmail(email) : undefined;
      window.requestAnimationFrame(() => {
        this._onChangeSignature(nextDefaultSignature);
      });
    }
  }

  componentWillUnmount() {
    this.unsubscribers.forEach((unsubscribe) => unsubscribe());
  }

  _getStateFromStores() {
    return {
      signatures: SignatureStore.getSignatures(),
    };
  }

  _onChangeSignature = (sig: ISignature | null | undefined) => {
    let body;
    if (sig) {
      body = applySignature(this.props.draft.body, sig);
    } else {
      body = applySignature(this.props.draft.body, null);
    }
    this.props.session.changes.add({ body });
  };

  _renderSignatures() {
    // note: these are using onMouseDown to avoid clearing focus in the composer (I think)
    let currentId: string | null;
    if (AppEnv.inSpecMode()) {
      currentId = currentSignatureId(this.props.draft.body);
    } else {
      currentId = currentSignatureIdSlate(this.props.draft.bodyEditorState);
    }

    return (
      <Menu
        headerComponents={[
          <MenuItem
            key={'none'}
            onMouseDown={() => this._onChangeSignature(null)}
            checked={!currentId}
            content={localized('No signature')}
          />,
        ]}
        footerComponents={this._staticFooterItems}
        items={Object.values(this.state.signatures)}
        itemKey={(sig) => sig.id}
        itemChecked={(sig) => currentId === sig.id}
        itemContent={(sig) => <span className={`signature-title-${sig.title}`}>{sig.title}</span>}
        onSelect={this._onChangeSignature}
      />
    );
  }

  render() {
    if (this.props.draft.plaintext) {
      return <span />;
    }
    return (
      <div className="signature-button-dropdown">
        <ButtonDropdown
          bordered={false}
          primaryItem={this._staticIcon}
          menu={this._renderSignatures()}
        />
      </div>
    );
  }
}
