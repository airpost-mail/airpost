import { AccountStore } from 'mailspring-exports';
import React from 'react';
import { sharedListener } from '../utils/shared-listener';

// Every thread row has one; they share an AccountStore listener.
let listenToAccounts: ReturnType<typeof sharedListener> = null;

class AccountColorBar extends React.Component<{ accountId: string }, { color: string | null }> {
  static displayName = 'AccountColorBar';

  unsubscribe?: () => void;

  constructor(props) {
    super(props);
    this.state = { color: this.getColor(props) };
  }

  getColor = (props = this.props) => {
    const account = AccountStore.accountForId(props.accountId);
    return account ? account.color : null;
  };

  componentDidMount() {
    listenToAccounts = listenToAccounts || sharedListener(AccountStore);
    this.unsubscribe = listenToAccounts(() => {
      const nextColor = this.getColor();
      if (this.state.color !== nextColor) this.setState({ color: nextColor });
    });
  }

  componentWillUnmount() {
    this.unsubscribe && this.unsubscribe();
  }

  render() {
    return this.state.color ? (
      <span
        style={{
          height: '50%',
          paddingLeft: '4px',
          borderLeftWidth: '4px',
          borderLeftColor: this.state.color,
          borderLeftStyle: 'solid',
        }}
      />
    ) : (
      <span />
    );
  }
}

export default AccountColorBar;
