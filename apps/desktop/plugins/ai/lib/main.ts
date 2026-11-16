// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

// AI features: Preferences > AI, the thread summary, reply
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
  PreferencesUIStoreTab,
} from 'mailspring-exports';
import { ThreadSummary, watchSummaries } from './thread-summary';
import { SuggestReplyButton } from './reply-suggestions';
import { TriageChip } from './triage-chip';
import { NeedsReplySidebarExtension } from './needs-reply';
import { moveKeysToEngine } from './client';

let preferencesTab: PreferencesUIStoreTab | null = null;
let stopWatchingSummaries: (() => void) | null = null;
let stopWatchingTriage: (() => void) | null = null;
let stopMovingKeys: (() => void) | null = null;

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

// The main window moves keys a build without engine-held keys left in the keyset.
function watchKeys() {
  const move = () =>
    moveKeysToEngine().catch((err) =>
      console.warn(`Could not move the AI keys to the engine: ${err}`)
    );
  Engine.events.on('connected', move);
  move();
  return () => Engine.events.removeListener('connected', move);
}

export function activate() {
  preferencesTab = new PreferencesUIStore.TabItem({
    tabId: 'AI',
    displayName: localized('AI'),
    componentClassFn: () => require('./preferences-ai').default,
    order: 20,
    group: 'connected',
    keywords:
      'ai model summary summaries triage reply suggestions ollama anthropic openai llm'.split(' '),
  });
  PreferencesUIStore.registerPreferencesTab(preferencesTab);
  ComponentRegistry.register(ThreadSummary, { role: 'MessageListHeaders' });
  stopWatchingSummaries = watchSummaries();
  ComponentRegistry.register(SuggestReplyButton, { role: 'Composer:ActionButton' });
  ComponentRegistry.register(TriageChip, { role: 'Thread:MailLabel' });
  stopWatchingTriage = watchTriage();
  if (AppEnv.isMainWindow()) stopMovingKeys = watchKeys();
}

export function deactivate() {
  ComponentRegistry.unregister(ThreadSummary);
  if (stopWatchingSummaries) stopWatchingSummaries();
  stopWatchingSummaries = null;
  ComponentRegistry.unregister(SuggestReplyButton);
  ComponentRegistry.unregister(TriageChip);
  if (stopWatchingTriage) stopWatchingTriage();
  stopWatchingTriage = null;
  if (stopMovingKeys) stopMovingKeys();
  stopMovingKeys = null;
  if (preferencesTab) PreferencesUIStore.unregisterPreferencesTab(preferencesTab.tabId);
  preferencesTab = null;
}
