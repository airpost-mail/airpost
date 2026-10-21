import { Attribute } from './attribute';
import * as Utils from '../models/utils';
import { Model } from '../models/model';
/*
Public: An object that can be cast to `itemClass`
Section: Database
*/
export class AttributeObject extends Attribute {
  private itemClass?: typeof Model;

  constructor({
    modelKey,
    jsonKey,
    itemClass,
    queryable,
  }: ConstructorParameters<typeof Attribute>[0] & { itemClass?: typeof Model }) {
    super({ modelKey, jsonKey, queryable });
    this.itemClass = itemClass;
  }

  toJSON(val: unknown) {
    return val && typeof val === 'object' && 'toJSON' in val && typeof val.toJSON === 'function'
      ? val.toJSON()
      : val;
  }

  fromJSON(val: unknown) {
    const Klass = this.itemClass;
    if (!val || (Klass && val instanceof Klass)) {
      return val;
    }
    if (Klass) {
      return new Klass(val);
    }
    if (typeof val === 'object' && '__cls' in val && val.__cls) {
      return Utils.convertToModel(val);
    }
    return val;
  }
}
