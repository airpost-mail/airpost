// Saves one draft to the engine (debounced) and remembers whether a copy exists there,
// so discarding or switching accounts can destroy it.
import { tasks, type EngineClient } from '@/engine';
import { createAutosave, type Autosave, type AutosaveStatus } from './autosave';
import { draftToMessage, type DraftState } from './draft';
import { queueTask } from './engine-calls';

interface SavedCopy {
  id: string;
  accountId: string;
}

export class DraftSaver {
  readonly autosave: Autosave<DraftState>;
  private copy: SavedCopy | null;
  private readonly client: EngineClient;

  constructor(
    client: EngineClient,
    opts: { existing?: SavedCopy | null; onStatus?: (s: AutosaveStatus) => void } = {},
  ) {
    this.client = client;
    this.copy = opts.existing ?? null;
    this.autosave = createAutosave({ save: (d) => this.save(d), onStatus: opts.onStatus });
  }

  async save(d: DraftState): Promise<void> {
    await queueTask(this.client, tasks.syncbackDraft(draftToMessage(d)));
    this.copy = { id: d.id, accountId: d.accountId };
  }

  get hasCopy(): boolean {
    return !!this.copy;
  }

  /** Queues a DestroyDraftTask for the saved copy, if any. Returns whether there was one. */
  destroyCopy(): boolean {
    const prev = this.copy;
    this.copy = null;
    if (prev) void queueTask(this.client, tasks.destroyDraft(prev.accountId, [prev.id]));
    return !!prev;
  }
}
