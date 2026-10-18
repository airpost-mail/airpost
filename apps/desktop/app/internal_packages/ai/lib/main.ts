// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

// AI features (docs/ai-plan.md section 7): Preferences > AI, the thread summary, reply
// suggestions, and triage (the chip in the thread list and "Needs reply" in the sidebar). The
// model calls run in the engine (`ai.*`); triage runs in the engine's own worker.

/* eslint global-require: 0 */
import {
  AI,
  Engine,
  ExtensionRegistry,
  localized,
  ComponentRegistry,
  PreferencesUIStore,
} from 'mailspring-exports';
import { ThreadSummary, watchSummaries } from './thread-summary';
import { SuggestReplyButton } from './reply-suggestions';
import { TriageChip } from './triage-chip';
import { NeedsReplySidebarExtension } from './needs-reply';

let preferencesTab = null;
let stopWatchingSummaries: (() => void) | null = null;
let stopWatchingTriage: (() => void) | null = null;

// Registering again makes the sidebar ask for its items again: "Needs reply" follows the
// triage switch, the opted-in accounts and the engine's features.
function refreshSidebar() {
  ExtensionRegistry.AccountSidebar.unregister(NeedsReplySidebarExtension);
  ExtensionRegistry.AccountSidebar.register(NeedsReplySidebarExtension);
}

function watchTriage() {
  ExtensionRegistry.AccountSidebar.register(NeedsReplySidebarExtension);
  const config = AppEnv.config.onDidChange(AI.CONFIG_KEY, refreshSidebar);
  Engine.events.on('connected', refreshSidebar);
  Engine.events.on('gone', refreshSidebar);
  return () => {
    config.dispose();
    Engine.events.removeListener('connected', refreshSidebar);
    Engine.events.removeListener('gone', refreshSidebar);
    ExtensionRegistry.AccountSidebar.unregister(NeedsReplySidebarExtension);
  };
}

export function activate() {
  preferencesTab = new PreferencesUIStore.TabItem({
    tabId: 'AI',
    displayName: localized('AI'),
    componentClassFn: () => require('./preferences-ai').default,
    order: 3.5,
  });
  PreferencesUIStore.registerPreferencesTab(preferencesTab);
  ComponentRegistry.register(ThreadSummary, { role: 'MessageListHeaders' });
  stopWatchingSummaries = watchSummaries();
  ComponentRegistry.register(SuggestReplyButton, { role: 'Composer:ActionButton' });
  ComponentRegistry.register(TriageChip, { role: 'Thread:MailLabel' });
  stopWatchingTriage = watchTriage();
}

export function deactivate() {
  ComponentRegistry.unregister(ThreadSummary);
  if (stopWatchingSummaries) stopWatchingSummaries();
  stopWatchingSummaries = null;
  ComponentRegistry.unregister(SuggestReplyButton);
  ComponentRegistry.unregister(TriageChip);
  if (stopWatchingTriage) stopWatchingTriage();
  stopWatchingTriage = null;
  if (preferencesTab) PreferencesUIStore.unregisterPreferencesTab(preferencesTab.tabId);
  preferencesTab = null;
}
