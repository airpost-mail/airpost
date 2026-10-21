import EventEmitter from 'events';

const MetadataChangePrefix = 'metadata.';

/**
Public: As the user interacts with the draft, changes are accumulated in the
DraftChangeSet associated with the store session.

This class used to be more complex - now it's mostly a holdover from when
we implemented undo/redo manually and just functions as a pass-through.

Section: Drafts
*/
const SaveAfterIdleMSec = 10000;
const SaveAfterIdleSlushMSec = 2000;

type DraftChangeSetCallbacks = {
  onAddChanges: (changes: Record<string, unknown>) => void;
  onCommit: () => Promise<void>;
};

export class DraftChangeSet extends EventEmitter {
  // Set by the constructor.
  callbacks!: DraftChangeSetCallbacks;
  _timer: ReturnType<typeof setTimeout> | null = null;
  _timerTime: number | null = null;
  _timerStarted: number | null = null;
  // Keyed by every changed field, plus body/bodyEditorState/pluginMetadata.
  _lastModifiedTimes: {
    [key: string]: number | undefined;
    body?: number;
    bodyEditorState?: number;
    pluginMetadata?: number;
  } = {};
  _lastCommitTime = 0;
  _commitPromise: Promise<any> | null = null;

  constructor(callbacks: DraftChangeSetCallbacks) {
    super();
    this.callbacks = callbacks;
  }

  clearDelayedCommit() {
    if (this._timer) {
      clearTimeout(this._timer);
      this._timerStarted = null;
      this._timer = null;
    }
  }

  add(changes: Record<string, unknown>, { skipSaving = false }: { skipSaving?: boolean } = {}) {
    if (!skipSaving) {
      changes.pristine = false;

      // update the per-attribute flags that track our dirty state
      for (const key of Object.keys(changes)) this._lastModifiedTimes[key] = performance.now();
      if (changes.bodyEditorState) this._lastModifiedTimes.body = performance.now();
      if (changes.body) this._lastModifiedTimes.bodyEditorState = performance.now();
      this.debounceCommit();
    }

    this.callbacks.onAddChanges(changes);
  }

  addPluginMetadata(pluginId: string, metadata: unknown) {
    this._lastModifiedTimes.pluginMetadata = performance.now();
    this.callbacks.onAddChanges({ [`${MetadataChangePrefix}${pluginId}`]: metadata });
    this.debounceCommit();
  }

  isDirty() {
    return this.dirtyFields().length > 0;
  }

  dirtyFields() {
    return Object.keys(this._lastModifiedTimes).filter(
      (key) => (this._lastModifiedTimes[key] ?? 0) > this._lastCommitTime
    );
  }

  debounceCommit() {
    const now = Date.now();

    // If there's already a timer going and we started it recently,
    // it means it'll fire a bit early but that's ok. It's actually
    // pretty expensive to re-create a timer on every keystroke.
    // _timerStarted is set whenever _timer is.
    if (this._timer && now - this._timerStarted! < SaveAfterIdleSlushMSec) {
      return;
    }
    this.clearDelayedCommit();
    this._timerStarted = now;
    this._timer = setTimeout(() => this.commit(), SaveAfterIdleMSec);
  }

  /*
  The `commit` method flushes the changes from the editor to SQLite. It's extremely important
  that this method does not resolve until everything has been saved. However:

  - A commit (triggered by a timer) may already be running when this function is called.
    We may not need to save again, but we need to block until that save completes.
  */
  async commit() {
    if (this._commitPromise) {
      await this._commitPromise;
    }

    if (this.dirtyFields().length > 0) {
      this.clearDelayedCommit();
      this._lastCommitTime = performance.now();
      this._commitPromise = this.callbacks.onCommit();
      await this._commitPromise;
      this._commitPromise = null;
    }
  }
}
