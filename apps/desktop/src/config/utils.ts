/* eslint-disable no-var */
/* eslint-disable prefer-rest-params */
import _ from 'underscore';

type PlainObject = { [key: string]: unknown };

function splitKeyPath(keyPath: string | null | undefined) {
  let _i;
  let startIndex = 0;
  const keyPathArray: string[] = [];
  if (keyPath == null) {
    return keyPathArray;
  }
  for (let i = (_i = 0); _i < keyPath.length; i = ++_i) {
    const char = keyPath[i];
    if (char === '.' && (i === 0 || keyPath[i - 1] !== '\\')) {
      keyPathArray.push(keyPath.substring(startIndex, i));
      startIndex = i + 1;
    }
  }
  keyPathArray.push(keyPath.substr(startIndex, keyPath.length));
  return keyPathArray;
}

function isPlainObject(value: unknown): value is PlainObject {
  return _.isObject(value) && !_.isArray(value);
}

export function remove<T>(array: T[], element: T) {
  const index = array.indexOf(element);
  if (index >= 0) {
    array.splice(index, 1);
  }
  return array;
}

export function deepClone(object: unknown): unknown {
  if (Array.isArray(object)) {
    return object.map(function (value): unknown {
      return deepClone(value);
    });
  } else if (_.isObject(object) && !_.isFunction(object)) {
    return _.mapObject(object, function (value): unknown {
      return deepClone(value);
    });
  } else {
    return object;
  }
}

export function deepExtend(...args: unknown[]) {
  let result = args[0];
  let i = 0;
  while (++i < args.length) {
    const object = args[i];
    if (isPlainObject(result) && isPlainObject(object)) {
      const _ref = Object.keys(object);
      for (let _i = 0; _i < _ref.length; _i++) {
        const key = _ref[_i];
        result[key] = deepExtend(result[key], object[key]);
      }
    } else {
      result = deepClone(object);
    }
  }
  return result;
}

export function valueForKeyPath(object: unknown, keyPath: string | null | undefined): unknown {
  const keys = splitKeyPath(keyPath);
  for (let _i = 0; _i < keys.length; _i++) {
    const key = keys[_i];
    // Throws on a non-object root, like the property access always did.
    object = (object as PlainObject)[key];
    if (object == null) {
      return;
    }
  }
  return object;
}

export function setValueForKeyPath(
  object: PlainObject,
  keyPath: string | null | undefined,
  value: unknown
) {
  const keys = splitKeyPath(keyPath);
  for (const key of keys.slice(0, -1)) {
    if (object[key] == null) {
      object[key] = {};
    }
    object = object[key] as PlainObject;
  }
  // A null keyPath has no keys, and writes the "undefined" key as it always did.
  const lastKey = String(keys[keys.length - 1]);
  if (value != null) {
    return (object[lastKey] = value);
  } else {
    return delete object[lastKey];
  }
}
