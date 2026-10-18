// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

// AI features (docs/ai-plan.md section 7): Preferences > AI, the thread summary and reply
// suggestions. The model calls run in the engine (`ai.*`); triage is stage AI4.

/* eslint global-require: 0 */
import { localized, ComponentRegistry, PreferencesUIStore } from 'mailspring-exports';
import { ThreadSummary } from './thread-summary';
import { SuggestReplyButton } from './reply-suggestions';

let preferencesTab = null;

export function activate() {
  preferencesTab = new PreferencesUIStore.TabItem({
    tabId: 'AI',
    displayName: localized('AI'),
    componentClassFn: () => require('./preferences-ai').default,
    order: 3.5,
  });
  PreferencesUIStore.registerPreferencesTab(preferencesTab);
  ComponentRegistry.register(ThreadSummary, { role: 'MessageListHeaders' });
  ComponentRegistry.register(SuggestReplyButton, { role: 'Composer:ActionButton' });
}

export function deactivate() {
  ComponentRegistry.unregister(ThreadSummary);
  ComponentRegistry.unregister(SuggestReplyButton);
  if (preferencesTab) PreferencesUIStore.unregisterPreferencesTab(preferencesTab.tabId);
  preferencesTab = null;
}
