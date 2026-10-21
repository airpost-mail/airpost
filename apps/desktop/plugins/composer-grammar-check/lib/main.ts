import { localized, Actions, ComponentRegistry, GrammarCheckPluginAPI } from 'mailspring-exports';
import { HasTutorialTip } from 'mailspring-component-kit';
import { GrammarCheckToggle } from './toggle';
import { GrammarCheckStore } from './store';

const { setGrammarCheckStore, clearGrammarCheckStore, cleanupDraft, clearAllGrammarDecorations } =
  GrammarCheckPluginAPI;

const GrammarCheckToggleWithTip = HasTutorialTip(GrammarCheckToggle, {
  title: localized('Check your grammar'),
  instructions: localized(
    'Enable grammar checking to find writing issues as you compose. Drafts are sent to the LanguageTool server set up in Preferences > Integrations.'
  ),
});

function _onDraftClosed(headerMessageId: string) {
  GrammarCheckStore.clearDraft(headerMessageId);
  cleanupDraft(headerMessageId);
}

// Module state, kept across activate/deactivate (previously on the module object via `this`).
let savedState: object = {};
let actionDisposables: (() => void)[] | null = null;
let configDisposable: { dispose: () => void } | null = null;

export function activate(state = {}) {
  savedState = state;

  GrammarCheckStore.activate();
  setGrammarCheckStore(GrammarCheckStore);

  actionDisposables = [
    Actions.sendDraft.listen((headerMessageId: string) => _onDraftClosed(headerMessageId)),
    Actions.destroyDraft.listen((draft: any) => {
      const id = typeof draft === 'string' ? draft : draft.headerMessageId;
      _onDraftClosed(id);
    }),
  ];

  configDisposable = AppEnv.config.onDidChange('core.composing.grammarCheck', ({ newValue }) => {
    if (!newValue) {
      clearAllGrammarDecorations();
    }
  });

  ComponentRegistry.register(GrammarCheckToggleWithTip, { role: 'Composer:ActionButton' });
}

export function deactivate() {
  // Remove any visible grammar underlines first, while the editor instances are
  // still reachable. The config-change listener handles the "feature toggled off"
  // path; this call covers the case where the package itself is deactivated while
  // the feature was still enabled.
  clearAllGrammarDecorations();

  // Disconnect the Slate plugin so no new decorations can be applied after this point.
  clearGrammarCheckStore();

  if (actionDisposables) {
    for (const unsub of actionDisposables) unsub();
    actionDisposables = null;
  }

  if (configDisposable) {
    configDisposable.dispose();
    configDisposable = null;
  }

  GrammarCheckStore.deactivate();
  ComponentRegistry.unregister(GrammarCheckToggleWithTip);
}

export function serialize() {
  return savedState;
}
