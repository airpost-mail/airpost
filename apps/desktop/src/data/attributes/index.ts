import { AttributeNumber } from './number';
import { AttributeString } from './string';
import { AttributeObject } from './object';
import { AttributeBoolean } from './boolean';
import { AttributeDateTime } from './datetime';
import { AttributeCollection } from './collection';
import { AttributeJoinedData } from './joined-data';

export * from './matcher';
export * from './sort-order';
export * from './number';
export * from './string';
export * from './object';
export * from './boolean';
export * from './datetime';
export * from './collection';
export * from './joined-data';

export function Number(options: ConstructorParameters<typeof AttributeNumber>[0]) {
  return new AttributeNumber(options);
}
export function String(options: ConstructorParameters<typeof AttributeString>[0]) {
  return new AttributeString(options);
}
export function Boolean(options: ConstructorParameters<typeof AttributeBoolean>[0]) {
  return new AttributeBoolean(options);
}
export function DateTime(options: ConstructorParameters<typeof AttributeDateTime>[0]) {
  return new AttributeDateTime(options);
}
export function Collection(options: ConstructorParameters<typeof AttributeCollection>[0]) {
  return new AttributeCollection(options);
}
export function JoinedData(options: ConstructorParameters<typeof AttributeJoinedData>[0]) {
  return new AttributeJoinedData(options);
}
export function Obj(options: ConstructorParameters<typeof AttributeObject>[0]) {
  return new AttributeObject(options);
}
