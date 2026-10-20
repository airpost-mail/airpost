import React from 'react';
import { localized, Actions, IAliasSet, ISignature, IDefaultSignatures } from 'mailspring-exports';
import { MultiselectDropdown } from 'mailspring-component-kit';

interface SignatureAccountDefaultPickerProps {
  defaults: IDefaultSignatures;
  signature: ISignature;
  accountsAndAliases: IAliasSet;
}

export default class SignatureAccountDefaultPicker extends React.Component<SignatureAccountDefaultPickerProps> {
  _onToggleAccount = (account: IAliasSet[0]) => {
    Actions.toggleAccount(account.email);
  };

  render() {
    const { accountsAndAliases, defaults, signature } = this.props;

    const isChecked = (accountOrAlias: IAliasSet[0]) =>
      defaults[accountOrAlias.email] === signature.id;
    const checked = accountsAndAliases.filter(isChecked);

    let buttonText: string;
    if (checked.length === 0) {
      buttonText = localized('No accounts');
    } else if (checked.length === 1) {
      buttonText = checked[0].email;
    } else {
      buttonText = `${checked.length} ${localized('accounts')}`;
    }

    return (
      <div className="account-picker">
        <MultiselectDropdown
          className="account-dropdown"
          items={accountsAndAliases}
          itemChecked={isChecked}
          itemContent={(accountOrAlias) => accountOrAlias.email}
          itemKey={(a) => a.id}
          attachment={'left'}
          buttonText={buttonText}
          onToggleItem={this._onToggleAccount}
        />
      </div>
    );
  }
}
