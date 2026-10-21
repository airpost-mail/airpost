const MAGIC_KEY = '__isPromisified__';
const IGNORED_PROPS = /^(?:length|name|arguments|caller|callee|prototype|__isPromisified__)$/;

/**
 * thatLooksLikeAPromiseToMe()
 *
 * Duck-types a promise.
 *
 * @param {object} o
 * @return {bool} True if this resembles a promise
 */
function thatLooksLikeAPromiseToMe(o: unknown) {
  if ((typeof o !== 'object' && typeof o !== 'function') || o === null) return false;
  return (
    typeof Reflect.get(o, 'then') === 'function' && typeof Reflect.get(o, 'catch') === 'function'
  );
}

function isPromisified(fn: unknown) {
  try {
    return Reflect.get(Object(fn), MAGIC_KEY) === true;
  } catch (e) {
    return false;
  }
}

type CallbackStyleFunction = (...args: unknown[]) => unknown;

/**
 * promisify()
 *
 * Transforms callback-based function -- func(arg1, arg2 .. argN, callback) -- into
 * an ES6-compatible Promise. Promisify provides a default callback of the form (error, result)
 * and rejects when `error` is truthy. You can also supply settings object as the second argument.
 *
 * @param {function} original - The function to promisify
 * @param {object} settings - Settings object
 * @param {object} settings.thisArg - A `this` context to use. If not set, assume `settings` _is_ `thisArg`
 * @param {bool} settings.multiArgs - Should multiple arguments be returned as an array?
 * @return {function} A promisified version of `original`
 */
function promisify(
  original: CallbackStyleFunction,
  settings?: { thisArg?: unknown; multiArgs?: boolean }
) {
  return function promiseWrapper(...args: unknown[]) {
    const returnMultipleArguments = settings && settings.multiArgs;
    let target: unknown;
    if (settings && settings.thisArg) {
      target = settings.thisArg;
    } else if (settings) {
      target = settings;
    }

    // Return the promisified function
    return new Promise(function promisified(resolve, reject) {
      // Append the callback bound to the context
      args.push(function callback(err: unknown, ...values: unknown[]) {
        if (err) {
          reject(err);
          return;
        }

        if (!!returnMultipleArguments === false) {
          resolve(values[0]);
          return;
        }

        resolve(values);
      });

      // Call the function
      const response = original.apply(target, args);

      // If it looks like original already returns a promise,
      // then just resolve with that promise. Hopefully, the callback function we added will just be ignored.
      if (thatLooksLikeAPromiseToMe(response)) {
        resolve(response);
      }
    });
  };
}

function promisifyAll(target: { [key: string]: unknown }) {
  Object.getOwnPropertyNames(target).forEach((key) => {
    const descriptor = Object.getOwnPropertyDescriptor(target, key);

    // A data property, so its value is target[key]
    const fn: unknown = descriptor && descriptor.value;
    if (typeof fn !== 'function') {
      return;
    }
    if (IGNORED_PROPS.test(key)) {
      return;
    }
    if (isPromisified(fn)) {
      return;
    }

    const promisifiedKey = `${key}Async`;

    target[promisifiedKey] = promisify(fn as CallbackStyleFunction, {});

    [key, promisifiedKey].forEach((rkey) => {
      Object.defineProperty(target[rkey], MAGIC_KEY, {
        value: true,
        configurable: true,
        enumerable: false,
        writable: true,
      });
    });
  });

  return target;
}

async function each<T, R>(items: Iterable<T>, fn: (item: T) => R | Promise<R>) {
  const results: R[] = [];
  for (const item of items) {
    results.push(await fn(item));
  }
  return results;
}

function delay(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function props(obj: { [key: string]: unknown }) {
  const awaitables: unknown[] = [];
  const keys = Object.keys(obj);
  for (const key of keys) {
    awaitables.push(obj[key]);
  }
  return Promise.all(awaitables).then(function r(results) {
    const byName: { [key: string]: unknown } = {};
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      byName[key] = results[i];
    }
    return byName;
  });
}

async function getState(this: Promise<unknown>) {
  const t = {};
  return await Promise.race([this, t]).then(
    (v) => (v === t ? 'pending' : 'fulfilled'),
    () => 'rejected'
  );
}

async function isResolved(this: Promise<unknown>) {
  return (await this.getState()) === 'fulfilled';
}

async function isRejected(this: Promise<unknown>) {
  return (await this.getState()) === 'rejected';
}

(global.Promise.prototype as any).getState = getState;
(global.Promise.prototype as any).isResolved = isResolved;
(global.Promise.prototype as any).isRejected = isRejected;

(global.Promise as any).each = each;
(global.Promise as any).delay = delay;
(global.Promise as any).props = props;
(global.Promise as any).promisify = promisify;
(global.Promise as any).promisifyAll = promisifyAll;

type ResolvableProps<T> = Record<string, unknown> & { [K in keyof T]: T[K] | Promise<T[K]> };
type IterateFunction<T, R> = (item: T, index: number, arrayLength: number) => Promise<R>;

interface PromiseConstructor {
  delay(ms: number): Promise<void>;
  props<T>(object: ResolvableProps<T>): Promise<any>;
  each<Q>(this: Promise<Iterable<Q>>, iterator: IterateFunction<Q, any>): Promise<any>;
  promisify(fn: any): any;
  promisifyAll(library: any): void;
}

interface Promise<T> {
  getState(): 'pending' | 'fulfilled' | 'rejected';
  isResolved(): Promise<boolean>;
  isRejected(): Promise<boolean>;
}
