import React from 'react';
import classNames from 'classnames';

import { localized, localizedReactFragment } from '../../../intl/localize';
import * as Actions from '../../../data/actions';
import { Account } from '../../../data/models/account';
import { AccountStore } from '../../../data/stores/account';
import MailRulesStore from '../../../data/stores/mail-rules';
import * as MailRulesTemplates from '../../../data/mail-rules-templates';

import ScenarioEditor from '../../../components/scenario-editor/scenario-editor';
import {
  SettingsPage,
  SettingsSection,
  SettingsRow,
  SettingsSwitch,
  SettingsField,
  StatusPill,
} from '../../../components/settings';
import { Button } from '../../../components/kit/button';
import { TextField } from '../../../components/kit/text-field';
import { SettingsSelect } from '../../../components/settings-select';

const { ActionTemplatesForAccount, ConditionTemplatesForAccount } = MailRulesTemplates;

type MailRule = ReturnType<typeof MailRulesStore.rulesForAccountId>[0];
type Templates = ReturnType<typeof ActionTemplatesForAccount>;

interface PreferencesMailRulesState {
  accounts: Account[];
  currentAccount: Account;
  rules: MailRule[];
  editingRuleId?: string;
  reprocessing?: ReturnType<typeof MailRulesStore.reprocessState>;
  actionTemplates: Templates;
  conditionTemplates: Templates;
}

/** One line for a rule's row: its conditions, an arrow, its actions. */
function summarize(rule: MailRule, conditionTemplates: Templates, actionTemplates: Templates) {
  const describe = (
    instance: { templateKey: string; comparatorKey?: string; value?: string; valueName?: string },
    templates: Templates
  ) => {
    const template = templates.find((t) => t.key === instance.templateKey);
    if (!template) return null;
    const comparator = instance.comparatorKey && template.comparators[instance.comparatorKey];
    let value = instance.valueName || instance.value;
    if (Array.isArray(template.values)) {
      const option = template.values.find((v) => v.value === instance.value);
      if (option) value = option.name;
    }
    if (template.type === 'None') return template.name;
    if (template.type === 'Enum' && !comparator) {
      return value ? `${template.name}: ${value}` : template.name;
    }
    return [template.name, comparator && comparator.name, value ? `"${value}"` : null]
      .filter(Boolean)
      .join(' ');
  };

  const joiner = rule.conditionMode === 'any' ? ` ${localized('or')} ` : ` ${localized('and')} `;
  const conditions = (rule.conditions || [])
    .map((c) => describe(c, conditionTemplates))
    .filter(Boolean)
    .join(joiner);
  const actions = (rule.actions || [])
    .map((a) => describe(a, actionTemplates))
    .filter(Boolean)
    .join(', ');
  return conditions && actions ? `${conditions} → ${actions}` : conditions || actions;
}

class PreferencesMailRules extends React.Component<
  Record<string, unknown>,
  PreferencesMailRulesState
> {
  static displayName = 'PreferencesMailRules';

  _unsubscribers = [];

  constructor(props) {
    super(props);
    this.state = this._getStateFromStores();
  }

  componentDidMount() {
    this._unsubscribers.push(MailRulesStore.listen(this._onRulesChanged));
  }

  componentWillUnmount() {
    this._unsubscribers.forEach((unsubscribe) => unsubscribe());
  }

  _getStateFromStores(): PreferencesMailRulesState {
    const accounts = AccountStore.accounts();

    let currentAccount = this.state ? this.state.currentAccount : null;
    if (!accounts.find((acct) => acct === currentAccount)) {
      currentAccount = accounts[0];
    }

    return {
      accounts: accounts,
      currentAccount: currentAccount,
      rules: currentAccount ? MailRulesStore.rulesForAccountId(currentAccount.id) : [],
      editingRuleId: this.state ? this.state.editingRuleId : undefined,
      reprocessing: MailRulesStore.reprocessState(),
      actionTemplates: ActionTemplatesForAccount(currentAccount),
      conditionTemplates: ConditionTemplatesForAccount(currentAccount),
    };
  }

  _onSelectAccount = (accountId: string) => {
    const currentAccount = this.state.accounts.find((acct) => acct.id === accountId);
    this.setState({ currentAccount: currentAccount, editingRuleId: undefined }, () => {
      this.setState(this._getStateFromStores());
    });
  };

  _onReprocessRules = () => {
    const needsMessageBodies = () => {
      for (const rule of this.state.rules) {
        for (const condition of rule.conditions) {
          if (condition.templateKey === 'body') {
            return true;
          }
        }
      }
      return false;
    };

    if (needsMessageBodies()) {
      AppEnv.showErrorDialog(
        localized(
          "One or more of your mail rules requires the bodies of messages being processed. These rules can't be run on your entire mailbox."
        )
      );
      return;
    }

    if (this.state.rules.length === 0) {
      AppEnv.showErrorDialog(
        localized(
          "You haven't created any mail rules. To get started, define a new rule above and tell Tern how to process your inbox."
        )
      );
      return;
    }

    if (!this.state.rules.some((r) => !r.disabled)) {
      AppEnv.showErrorDialog(
        localized(
          'All of the mail rules for this account are disabled. Re-enable at least one rule before processing your inbox.'
        )
      );
      return;
    }
    Actions.startReprocessingMailRules(this.state.currentAccount.id);
  };

  _onAddRule = () => {
    Actions.addMailRule({ accountId: this.state.currentAccount.id });
  };

  _onToggleEditing = (rule: MailRule) => {
    this.setState({ editingRuleId: this.state.editingRuleId === rule.id ? undefined : rule.id });
  };

  /** Moves a rule past its neighbour in this account's list; the store orders all accounts' rules together. */
  _onMoveRule = (rule: MailRule, by: -1 | 1) => {
    const { rules } = this.state;
    const neighbour = rules[rules.findIndex((r) => r.id === rule.id) + by];
    if (!neighbour) return;
    const all = MailRulesStore.rules();
    Actions.reorderMailRule(
      rule.id,
      all.findIndex((r) => r.id === neighbour.id)
    );
  };

  _onDeleteRule = (rule: MailRule) => {
    Actions.deleteMailRule(rule.id);
  };

  _onRuleNameEdited = (newName: string, rule: MailRule) => {
    if (newName !== rule.name) {
      Actions.updateMailRule(rule.id, { name: newName });
    }
  };

  _onSetRuleEnabled = (rule: MailRule, enabled: boolean) => {
    Actions.updateMailRule(
      rule.id,
      enabled ? { disabled: false, disabledReason: null } : { disabled: true }
    );
  };

  _onRulesChanged = () => {
    const next = this._getStateFromStores();
    const prevRules = this.state.rules ? this.state.rules : [];

    const added = next.rules.filter((r) => !prevRules.some((p) => p.id === r.id));
    if (added.length === 1 && prevRules.length + 1 === next.rules.length) {
      next.editingRuleId = added[0].id;
    }

    this.setState(next);
  };

  _renderAccountPicker() {
    if (this.state.accounts.length < 2) return null;
    return (
      <SettingsSection>
        <SettingsRow
          label={localized('Account')}
          description={localized('Each account has its own rules.')}
        >
          <SettingsSelect
            aria-label={localized('Account')}
            value={this.state.currentAccount.id}
            onChange={this._onSelectAccount}
            options={this.state.accounts.map((account): [string, string] => [
              account.id,
              account.label,
            ])}
          />
        </SettingsRow>
      </SettingsSection>
    );
  }

  _renderRule = (rule: MailRule, idx: number) => {
    const editing = this.state.editingRuleId === rule.id;
    const summary = summarize(rule, this.state.conditionTemplates, this.state.actionTemplates);
    return (
      <div key={rule.id} className={classNames('mail-rule', { editing, disabled: rule.disabled })}>
        <SettingsRow label={rule.name} description={summary}>
          {rule.disabled && rule.disabledReason && (
            <StatusPill kind="bad">{localized('Stopped')}</StatusPill>
          )}
          <Button variant="outline" onClick={() => this._onToggleEditing(rule)}>
            {editing ? localized('Done') : localized('Edit')}
          </Button>
          <SettingsSwitch
            checked={!rule.disabled}
            label={localized('Turn on %@', rule.name)}
            onChange={(on) => this._onSetRuleEnabled(rule, on)}
          />
        </SettingsRow>
        {editing && this._renderEditor(rule, idx)}
      </div>
    );
  };

  _renderEditor(rule: MailRule, idx: number) {
    return (
      <div className="rule-detail">
        {rule.disabled && rule.disabledReason && (
          <div className="disabled-reason">
            <div>
              {localized(
                'This rule stopped because of an error. Check the actions below, then turn it back on.'
              )}
              <div className="disabled-reason-detail">{rule.disabledReason}</div>
            </div>
            <Button variant="outline" onClick={() => this._onSetRuleEnabled(rule, true)}>
              {localized('Turn on')}
            </Button>
          </div>
        )}
        <div className="inner">
          <SettingsField label={localized('Name')} htmlFor={`mail-rule-name-${rule.id}`}>
            <TextField
              id={`mail-rule-name-${rule.id}`}
              key={rule.id}
              defaultValue={rule.name}
              onBlur={(e) => this._onRuleNameEdited(e.target.value, rule)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
              }}
            />
          </SettingsField>
          <div className="mail-rule-label">
            {localizedReactFragment(
              'If %@ of the following conditions are met:',
              <SettingsSelect
                key="condition-mode"
                size="sm"
                aria-label={localized('Any or all')}
                value={rule.conditionMode}
                onChange={(conditionMode: MailRule['conditionMode']) =>
                  Actions.updateMailRule(rule.id, { conditionMode })
                }
                options={[
                  ['any', localized('Any')],
                  ['all', localized('All')],
                ]}
              />
            )}
          </div>
          <ScenarioEditor
            instances={rule.conditions}
            templates={this.state.conditionTemplates}
            onChange={(conditions) => Actions.updateMailRule(rule.id, { conditions })}
            className="well well-matchers"
          />
          <div className="mail-rule-label">{localized('Perform these actions:')}</div>
          <ScenarioEditor
            instances={rule.actions}
            templates={this.state.actionTemplates}
            onChange={(actions) => Actions.updateMailRule(rule.id, { actions })}
            className="well well-actions"
          />
          <div className="settings-actions">
            <Button
              variant="outline"
              disabled={idx === 0}
              onClick={() => this._onMoveRule(rule, -1)}
            >
              {localized('Move up')}
            </Button>
            <Button
              variant="outline"
              disabled={idx === this.state.rules.length - 1}
              onClick={() => this._onMoveRule(rule, 1)}
            >
              {localized('Move down')}
            </Button>
            <span className="spacer" />
            <Button variant="destructive" onClick={() => this._onDeleteRule(rule)}>
              {localized('Delete rule')}
            </Button>
          </div>
        </div>
      </div>
    );
  }

  _renderRules() {
    const note = localizedReactFragment(
      'Rules run on this computer while Tern is open, in the order shown. To run them after you close the window, %@.',
      <a onClick={() => Actions.switchPreferencesTab('General')}>
        {localized('keep Tern running')}
      </a>
    );
    return (
      <SettingsSection className="mail-rules-list" note={note}>
        {this.state.rules.length === 0 && (
          <div className="mail-rules-empty">{localized('No rules for this account yet.')}</div>
        )}
        {this.state.rules.map(this._renderRule)}
        <Button variant="ghost" className="mail-rules-new" onClick={this._onAddRule}>
          {localized('+ New rule')}
        </Button>
      </SettingsSection>
    );
  }

  _renderReprocessing() {
    const { reprocessing, currentAccount } = this.state;
    return (
      <SettingsSection title={localized('Mail you already have')}>
        <SettingsRow
          label={localized('Run rules on your inbox')}
          description={localized(
            'Rules only apply to new mail as it arrives. Running them on your whole inbox can take a long time and slow Tern down.'
          )}
        >
          <Button
            variant="outline"
            disabled={!!reprocessing[currentAccount.id]}
            onClick={this._onReprocessRules}
          >
            {localized('Run now')}
          </Button>
        </SettingsRow>
        {Object.keys(reprocessing).map((accountId) => {
          const { count, total } = reprocessing[accountId];
          const account = AccountStore.accountForId(accountId);
          return (
            <SettingsRow
              key={accountId}
              label={account ? account.emailAddress : accountId}
              description={
                total
                  ? localized(
                      '%@ of %@ processed',
                      Number(count).toLocaleString(),
                      Number(total).toLocaleString()
                    )
                  : localized('Starting…')
              }
            >
              <Button
                variant="outline"
                onClick={() => Actions.stopReprocessingMailRules(accountId)}
              >
                {localized('Stop')}
              </Button>
            </SettingsRow>
          );
        })}
      </SettingsSection>
    );
  }

  render() {
    return (
      <SettingsPage
        title={localized('Rules')}
        description={localized('Sort mail automatically as it arrives.')}
        className="container-mail-rules"
      >
        {this.state.currentAccount ? (
          <>
            {this._renderAccountPicker()}
            {this._renderRules()}
            {this._renderReprocessing()}
          </>
        ) : (
          <SettingsSection>
            <div className="mail-rules-empty">{localized('Add an account to make rules.')}</div>
          </SettingsSection>
        )}
      </SettingsPage>
    );
  }
}

export default PreferencesMailRules;
