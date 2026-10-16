import type { Delta, ModelClass, ModelJSONByClass } from '../types';

/** Collects one task's writes and turns them into coalesced deltas, like `DeltaStream`. */
export class ChangeSet {
  private persisted = new Map<ModelClass, Map<string, unknown>>();
  private unpersisted = new Map<ModelClass, Map<string, unknown>>();

  constructor(readonly accountId: string) {}

  persist<C extends ModelClass>(cls: C, json: ModelJSONByClass[C] & { id?: string }) {
    const id = (json as { id?: string }).id ?? String(this.persisted.get(cls)?.size ?? 0);
    this.unpersisted.get(cls)?.delete(id);
    this.bucket(this.persisted, cls).set(id, structuredClone(json));
  }

  unpersist<C extends ModelClass>(cls: C, json: ModelJSONByClass[C] & { id: string }) {
    this.persisted.get(cls)?.delete(json.id);
    this.bucket(this.unpersisted, cls).set(json.id, structuredClone(json));
  }

  get empty(): boolean {
    return this.toDeltas().length === 0;
  }

  merge(other: ChangeSet) {
    for (const d of other.toDeltas()) {
      for (const json of d.modelJSONs) {
        if (d.type === 'persist') this.persist(d.modelClass, json as never);
        else this.unpersist(d.modelClass, json as never);
      }
    }
  }

  toDeltas(): Delta[] {
    const out: Delta[] = [];
    for (const [type, map] of [
      ['persist', this.persisted],
      ['unpersist', this.unpersisted],
    ] as const) {
      for (const [modelClass, byId] of map) {
        if (byId.size === 0) continue;
        out.push({
          accountId: this.accountId,
          modelClass,
          type,
          modelJSONs: [...byId.values()] as Delta['modelJSONs'],
        });
      }
    }
    return out;
  }

  private bucket(map: Map<ModelClass, Map<string, unknown>>, cls: ModelClass) {
    let b = map.get(cls);
    if (!b) {
      b = new Map();
      map.set(cls, b);
    }
    return b;
  }
}
