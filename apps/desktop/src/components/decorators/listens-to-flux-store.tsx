/* eslint no-prototype-builtins: 0 */
import React, { Component } from 'react';

interface ListenableStore {
  listen(callback: () => void): () => void;
}

// This HOC is untyped by design: it passes whatever getStateFromStores returns into
// whatever it wraps (class or function component, which may return false), and returns
// `any`. These `any`s were implicit before; typing them rejects existing callers whose
// props and state don't line up exactly.
type Props = any;
type State = object;

function ListensToFluxStore(
  ComposedComponent: any,
  {
    stores,
    getStateFromStores,
  }: { stores: ListenableStore[]; getStateFromStores: (props: Props) => State }
) {
  return class extends Component<Props, State> {
    static displayName = ComposedComponent.displayName;

    static containerRequired = false;

    _unlisteners: Array<() => void> = [];
    _composedComponent: unknown;

    constructor(props: Props) {
      super(props);
      this.state = getStateFromStores(props);
    }

    componentDidMount() {
      stores.forEach((store) => {
        this._unlisteners.push(
          store.listen(() => {
            this.setState(getStateFromStores(this.props));
          })
        );
      });
    }

    componentDidUpdate(prevProps: Props) {
      if (prevProps !== this.props) {
        this.setState(getStateFromStores(this.props));
      }
    }

    componentWillUnmount() {
      for (const unlisten of this._unlisteners) {
        unlisten();
      }
      this._unlisteners = [];
    }

    render() {
      const props: Props = {
        ...this.props,
        ...this.state,
      };
      if (Component.isPrototypeOf(ComposedComponent)) {
        props.ref = (cm: unknown) => {
          this._composedComponent = cm;
        };
      }
      return <ComposedComponent {...props} />;
    }
  } as any;
}

export default ListensToFluxStore;
