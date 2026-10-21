import SerializableRegistry from './serializable-registry';
import type { Model } from './models/model';

class DatabaseObjectRegistry extends SerializableRegistry<Model> {}

const registry = new DatabaseObjectRegistry();
export default registry;
