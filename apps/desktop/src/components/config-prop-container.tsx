import React from 'react';
import { Disposable } from 'rx-core';

export class ConfigPropContainer extends React.Component<
  {
    children: React.ReactElement<any>;
  },
  { config: any }
> {
  static displayName = 'ConfigPropContainer';

  subscription?: Disposable;

  constructor(props: { children: React.ReactElement<any> }) {
    super(props);
    this.state = this.getStateFromStores();
  }

  componentDidMount() {
    this.subscription = AppEnv.config.onDidChange(null, () => {
      this.setState(this.getStateFromStores());
    });
  }

  componentWillUnmount() {
    if (this.subscription) {
      this.subscription.dispose();
    }
  }

  getStateFromStores() {
    return {
      config: this.getConfigWithMutators(),
    };
  }

  getConfigWithMutators() {
    return Object.assign(AppEnv.config.get(), {
      get: (key: string) => {
        return AppEnv.config.get(key);
      },
      set: (key: string, value: unknown) => {
        AppEnv.config.set(key, value);
      },
      toggle: (key: string) => {
        AppEnv.config.set(key, !AppEnv.config.get(key));
      },
      contains: (key: string, val: unknown) => {
        const vals = AppEnv.config.get(key);
        return vals && vals instanceof Array ? vals.includes(val) : false;
      },
      toggleContains: (key: string, val: unknown) => {
        let vals = AppEnv.config.get(key);
        if (!vals || !(vals instanceof Array)) {
          vals = [];
        }
        if (vals.includes(val)) {
          AppEnv.config.set(
            key,
            vals.filter((v: unknown) => v !== val)
          );
        } else {
          AppEnv.config.set(key, vals.concat([val]));
        }
      },
    });
  }

  render() {
    return React.cloneElement(this.props.children, {
      config: this.state.config,
      configSchema: AppEnv.config.getSchema('core'),
    });
  }
}
