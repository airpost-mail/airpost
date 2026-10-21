import { Task } from './task';
import DatabaseObjectRegistry from '../database-object-registry';

export class DestroyModelTask extends Task {
  modelId?: string;
  endpoint?: string;
  modelName?: string;

  constructor({
    modelId,
    modelName,
    endpoint,
    accountId,
  }: { modelId?: string; modelName?: string; endpoint?: string; accountId?: string } = {}) {
    super({ accountId });
    this.modelId = modelId;
    this.endpoint = endpoint;
    this.modelName = modelName;
  }

  shouldDequeueOtherTask(other: Task) {
    return (
      other instanceof DestroyModelTask &&
      this.modelName === other.modelName &&
      this.accountId === other.accountId &&
      this.endpoint === other.endpoint &&
      this.modelId === other.modelId
    );
  }

  getModelConstructor() {
    if (!this.modelName) {
      throw new Error('DestroyModelTask: no modelName');
    }
    return DatabaseObjectRegistry.get(this.modelName);
  }
}
