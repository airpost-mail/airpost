import type { Model } from './model';

// A model holds each attribute's value in a plain field named by the attribute's
// `modelKey`. These read and write that field for code that walks `attributes` by name.
export function attributeValue(model: Model, key: string): unknown {
  return (model as unknown as Record<string, unknown>)[key];
}

// What a sortable or range-matched attribute holds. The relational operators read these
// with JS coercion, as the untyped code always did (a null value compares as 0).
export type ComparableValue = number | string | Date | boolean;

export function setAttributeValue(model: Model, key: string, value: unknown) {
  (model as unknown as Record<string, unknown>)[key] = value;
}
