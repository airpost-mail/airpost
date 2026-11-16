// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

import React from 'react';
import { Actions, localized, Thread } from 'mailspring-exports';
import { MailLabel, Menu } from 'mailspring-component-kit';
import {
  CATEGORIES,
  TriageCategory,
  TriageValue,
  categoryLabel,
  chipCategory,
  correctCategory,
  triageOf,
} from './triage';

const HUES = { 'needs-reply': 28, important: 0 };

/** The correction menu: the model's reason as its header, then every category, the current one checked. */
export function CorrectionMenu({ thread, value }: { thread: Thread; value: TriageValue }) {
  const header =
    value.reason && value.source !== 'user' ? (
      <span key="reason" className="ai-triage-reason">
        {value.reason}
      </span>
    ) : (
      <span key="title">{localized('Category')}:</span>
    );
  return (
    <Menu
      className="ai-triage-menu"
      items={CATEGORIES}
      itemKey={(c: TriageCategory) => c}
      itemContent={(c: TriageCategory) => categoryLabel(c)}
      itemChecked={(c: TriageCategory) => c === value.category}
      headerComponents={[header]}
      defaultSelectedIndex={-1}
      onSelect={(c: TriageCategory) => {
        Actions.closePopover();
        if (c !== value.category || value.source !== 'user') correctCategory(thread, c);
      }}
      onEscape={() => Actions.closePopover()}
    />
  );
}

/** The triage chip in the thread list (role Thread:MailLabel): needs-reply and important only. */
export class TriageChip extends React.Component<{ thread: Thread }> {
  static displayName = 'TriageChip';
  static containerRequired = false;

  _onMenu = (event: React.MouseEvent<HTMLElement>) => {
    event.preventDefault();
    event.stopPropagation();
    const value = triageOf(this.props.thread);
    if (!value) return;
    Actions.openPopover(<CorrectionMenu thread={this.props.thread} value={value} />, {
      originRect: event.currentTarget.getBoundingClientRect(),
      direction: 'down',
    });
  };

  render() {
    const value = triageOf(this.props.thread);
    const category = chipCategory(value, this.props.thread);
    if (!value || !category) return null;
    const label = {
      displayName: categoryLabel(category),
      isLockedCategory: () => true,
      hue: () => HUES[category],
    };
    const title =
      value.source === 'user'
        ? localized('You filed this. Click to change.')
        : localized('Sorted by AI. Click to change.');
    return (
      <span className={`ai-triage-chip ai-triage-${category}`} title={title} onClick={this._onMenu}>
        <MailLabel label={label} />
      </span>
    );
  }
}
