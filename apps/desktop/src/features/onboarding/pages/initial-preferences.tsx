import React from 'react';
import path from 'path';
import fs from 'fs';
import classnames from 'classnames';
import { RetinaImg } from '../../../components/retina-img';
import { ConfigPropContainer } from '../../../components/config-prop-container';
import { localized } from '../../../intl/localize';
import { AccountStore } from '../../../data/stores/account';
import { Account } from '../../../data/models/account';
import { Button } from '../../../components/kit/button';
import { RadioGroup, RadioGroupItem } from '../../../components/kit/radio-group';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../../components/kit/select';

// One tile of the layout choice; `active` stays on the checked tile for themes.
const AppearanceModeOption = (props: { mode: string; active: boolean }) => {
  const label = {
    list: localized('Reading Pane Off'),
    split: localized('Reading Pane On'),
  }[props.mode];

  return (
    <RadioGroupItem
      value={props.mode}
      aria-label={label}
      className={classnames('appearance-mode', { active: props.active })}
    >
      <RetinaImg name={`appearance-mode-${props.mode}.png`} mode={RetinaImg.Mode.ContentIsMask} />
      <div>{label}</div>
    </RadioGroupItem>
  );
};

class InitialPreferencesOptions extends React.Component<
  { account: Account; config?: any },
  { templates: any[] }
> {
  constructor(props: { account: Account; config?: any }) {
    super(props);
    this.state = { templates: [] };
    this._loadTemplates();
  }

  _loadTemplates = () => {
    const templatesDir = path.join(AppEnv.getLoadSettings().resourcePath, 'keymaps', 'templates');
    fs.readdir(templatesDir, (err, files) => {
      if (!files || !(files instanceof Array)) {
        return;
      }
      let templates = files.filter(
        (filename) => path.extname(filename) === '.cson' || path.extname(filename) === '.json'
      );
      templates = templates.map((filename) => path.parse(filename).name);
      this.setState({ templates });
      this._setConfigDefaultsForAccount(templates);
    });
  };

  _setConfigDefaultsForAccount = (templates: string[]) => {
    if (!this.props.account) {
      return;
    }

    const templateWithBasename = (name: string) => templates.find((t) => t.indexOf(name) === 0);

    if (this.props.account.provider === 'gmail') {
      this.props.config.set('core.workspace.mode', 'list');
      this.props.config.set('core.keymapTemplate', templateWithBasename('Gmail'));
    } else if (
      this.props.account.provider === 'eas' ||
      this.props.account.provider === 'office365' ||
      this.props.account.provider === 'outlook'
    ) {
      this.props.config.set('core.workspace.mode', 'split');
      this.props.config.set('core.keymapTemplate', templateWithBasename('Outlook'));
    } else {
      this.props.config.set('core.workspace.mode', 'split');
      if (process.platform === 'darwin') {
        this.props.config.set('core.keymapTemplate', templateWithBasename('Apple Mail'));
      } else {
        this.props.config.set('core.keymapTemplate', templateWithBasename('Outlook'));
      }
    }
  };

  render() {
    if (!this.props.config) {
      return false;
    }

    return (
      <div
        style={{
          display: 'flex',
          width: 600,
          marginBottom: 50,
          marginLeft: 150,
          marginRight: 150,
          textAlign: 'left',
        }}
      >
        <div style={{ flex: 1 }}>
          <p>
            {localized('Do you prefer a single panel layout (like Gmail) or a two panel layout?')}
          </p>
          <RadioGroup
            className="appearance-mode-switch"
            orientation="horizontal"
            aria-label={localized('Layout')}
            value={this.props.config.get('core.workspace.mode')}
            onValueChange={(mode) => this.props.config.set('core.workspace.mode', mode)}
          >
            {['list', 'split'].map((mode) => (
              <AppearanceModeOption
                mode={mode}
                key={mode}
                active={this.props.config.get('core.workspace.mode') === mode}
              />
            ))}
          </RadioGroup>
        </div>
        <div
          key="divider"
          style={{ marginLeft: 20, marginRight: 20, borderLeft: '1px solid #ccc' }}
        />
        <div style={{ flex: 1 }}>
          <p>
            {localized(
              `We've picked a set of keyboard shortcuts based on your email account and platform. You can also pick another set:`
            )}
          </p>
          <Select
            value={this.props.config.get('core.keymapTemplate')}
            onValueChange={(template) => this.props.config.set('core.keymapTemplate', template)}
          >
            <SelectTrigger aria-label={localized('Shortcuts')}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {this.state.templates.map((template) => (
                <SelectItem key={template} value={template}>
                  {template}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
    );
  }
}

class InitialPreferencesPage extends React.Component<
  Record<string, unknown>,
  { account: Account }
> {
  static displayName = 'InitialPreferencesPage';

  _unlisten?: () => void;

  constructor(props: Record<string, unknown>) {
    super(props);
    this.state = { account: AccountStore.accounts()[0] };
  }

  componentDidMount() {
    this._unlisten = AccountStore.listen(this._onAccountStoreChange);
  }

  componentWillUnmount() {
    if (this._unlisten) {
      this._unlisten();
    }
  }

  _onAccountStoreChange = () => {
    this.setState({ account: AccountStore.accounts()[0] });
  };

  render() {
    if (!this.state.account) {
      return <div />;
    }
    return (
      <div className="page opaque" style={{ width: 900, height: 620 }}>
        <h1 style={{ paddingTop: 100 }}>{localized(`Welcome to Tern`)}</h1>
        <h4 style={{ marginBottom: 60 }}>{localized(`Let's set things up to your liking.`)}</h4>
        <ConfigPropContainer>
          <InitialPreferencesOptions account={this.state.account} />
        </ConfigPropContainer>
        <Button variant="outline" size="lg" style={{ marginBottom: 60 }} onClick={this._onFinished}>
          {localized(`Looks Good!`)}
        </Button>
      </div>
    );
  }

  _onFinished = () => {
    require('electron').ipcRenderer.send('account-setup-successful');
  };
}

export default InitialPreferencesPage;
