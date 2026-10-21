/* eslint-disable no-var */
/*
 * decaffeinate suggestions:
 * DS207: Consider shorter variations of null checks
 * Full docs: https://github.com/decaffeinate/decaffeinate/blob/master/docs/suggestions.md
 */
import _ from 'underscore';
import fs from 'fs';
import path from 'path';

import DatabaseObjectRegistry from '../database-object-registry';
import { escapeRegExp } from '../../utils/regexp';

export function waitFor(latch: () => unknown, options: { timeout?: number } = {}) {
  const timeout = options.timeout || 400;
  const expire = Date.now() + timeout;
  return new Promise<void>(function (resolve, reject) {
    const attempt = () => {
      if (Date.now() > expire) {
        return reject(new Error(`Utils.waitFor hit timeout (${timeout}ms) without firing.`));
      }
      if (latch()) {
        return resolve();
      }
      window.requestAnimationFrame(attempt);
    };
    attempt();
  });
}

export function showIconForAttachments(files: any[]) {
  if (!(files instanceof Array)) {
    return false;
  }
  // TODO BG: This code has been duplicated into the engine. The
  // Thread.attachmentCount property is now the number of attachments that
  // meet these two criteria so this function can be removed soon.
  return files.find((f) => !f.contentId || f.size > 12 * 1024);
}

export function extractTextFromHtml(
  html: string | null | undefined,
  param: { maxLength?: number } = {}
) {
  const { maxLength } = param;
  if ((html != null ? html : '').trim().length === 0) {
    return '';
  }
  let source = html || '';
  if (maxLength && source.length > maxLength) {
    source = source.slice(0, maxLength);
  }
  const body = new DOMParser().parseFromString(source, 'text/html').body;
  body.querySelectorAll('style').forEach((el) => el.remove());
  body.querySelectorAll('script').forEach((el) => el.remove());
  // An element's textContent is never null.
  return (body.textContent || '').trim();
}

export function modelTypesReviver(k: string, v: any) {
  const type = v != null ? v.__cls : undefined;
  if (!type) {
    return v;
  }

  if (DatabaseObjectRegistry.isInRegistry(type)) {
    return DatabaseObjectRegistry.deserialize(type, v);
  }

  return v;
}

export function convertToModel(json: any) {
  if (!json) {
    return null;
  }
  if (!json.__cls) {
    throw new Error('convertToModel: no __cls found on object.');
  }
  if (!DatabaseObjectRegistry.isInRegistry(json.__cls)) {
    throw new Error('convertToModel: __cls is not a known class.');
  }
  return DatabaseObjectRegistry.deserialize(json.__cls, json);
}

export function fastOmit(props: Record<string, any>, without: string[]) {
  const otherProps = Object.assign({}, props);
  for (const w of without) {
    delete otherProps[w];
  }
  return otherProps;
}

export function isHash(object: any) {
  return _.isObject(object) && !_.isFunction(object) && !_.isArray(object);
}

export { escapeRegExp };

export function range(left: number, right: number, inclusive = true) {
  const range: number[] = [];
  const ascending = left < right;
  const end = !inclusive ? right : ascending ? right + 1 : right - 1;
  for (let i = left; ascending ? i < end : i > end; ascending ? i++ : i--) {
    range.push(i);
  }
  return range;
}

// Generates a new RegExp that is great for basic search fields. It
// checks if the test string is at the start of words.
//
// Note: the `g` flag is intentionally NOT set. Callers use this regex
// with `.test()` (which mutates `lastIndex` on global regexes, leading
// to subtle bugs when a single instance is reused across items) and
// with `String.prototype.split()` (which does not require `g` to split
// on every occurrence). Adding `g` here would silently re-introduce
// the lastIndex hazard for future callers.
//
// See regex explanation and test here:
// https://regex101.com/r/zG7aW4/2
export function wordSearchRegExp(str = '') {
  return new RegExp(`((?:^|\\W|$)${escapeRegExp(str.trim())})`, 'i');
}

// Takes an optional customizer. The customizer is passed the key and the
// new cloned value for that key. The customizer is expected to either
// modify the value and return it or simply be the identity function.
export function deepClone<T>(
  object: T,
  // Plugin API: the value was untyped before, and customizers read it as whatever they cloned.
  customizer?: (key: string, value: any) => unknown,
  _circularRefs: Map<any, any> = new Map()
): T {
  let newObject: object;
  if (!_.isObject(object)) {
    return object;
  }
  if (_.isFunction(object)) {
    return object;
  }

  if (_.isArray(object)) {
    // http://perfectionkills.com/how-ecmascript-5-still-does-not-allow-to-subclass-an-array/
    newObject = [];
  } else if (object instanceof Date) {
    // You can't clone dates by iterating through `getOwnPropertyNames`
    // of the Date object. We need to special-case Dates.
    newObject = new Date(object);
  } else {
    newObject = Object.create(Object.getPrototypeOf(object));
  }

  // Circular reference check using Map for O(1) lookup instead of O(n) indexOf
  if (_circularRefs.has(object)) {
    return _circularRefs.get(object);
  }
  _circularRefs.set(object, newObject);

  // Copying walks the properties by name, whatever the object's type.
  const source = object as Record<string, unknown>;
  const target = newObject as Record<string, unknown>;
  // It's important to use getOwnPropertyNames instead of Object.keys to
  // get the non-enumerable items as well.
  for (const key of Object.getOwnPropertyNames(object)) {
    const newVal = deepClone(source[key], customizer, _circularRefs);
    if (_.isFunction(customizer)) {
      target[key] = customizer(key, newVal);
    } else {
      target[key] = newVal;
    }
  }
  return newObject as T;
}

export function toSet(arr: any[] = []) {
  const set: { [item: string]: boolean } = {};
  for (const item of arr) {
    set[item] = true;
  }
  return set;
}

// Given a File object or uploadData of an uploading file object,
// determine if it looks like an image and is in the size range for previews
export function shouldDisplayAsImage(
  file: {
    filename?: string;
    fileName?: string;
    name?: string;
    size?: number;
    fileSize?: number;
  } = {}
) {
  const name = file.filename || file.fileName || file.name || '';
  const size = file.size || file.fileSize || 0;
  const ext = path.extname(name).toLowerCase();
  const extensions = ['.jpg', '.bmp', '.gif', '.png', '.jpeg', '.webp'];

  return extensions.includes(ext) && size > 512 && size < 1024 * 1024 * 5;
}

// Escapes potentially dangerous html characters
// This code is lifted from Angular.js
// See their specs here:
// https://github.com/angular/angular.js/blob/master/test/ngSanitize/sanitizeSpec.js
// And the original source here: https://github.com/angular/angular.js/blob/master/src/ngSanitize/sanitize.js#L451
// AngularJS is Copyright (c) 2010-2020 Google LLC, MIT licence: https://github.com/angular/angular.js/blob/master/LICENSE
export function encodeHTMLEntities(value: string) {
  const SURROGATE_PAIR_REGEXP = /[\uD800-\uDBFF][\uDC00-\uDFFF]/g;
  const pairFix = function (value: string) {
    const hi = value.charCodeAt(0);
    const low = value.charCodeAt(1);
    return `&#${(hi - 0xd800) * 0x400 + (low - 0xdc00) + 0x10000};`;
  };

  // Match everything outside of normal chars and " (quote character)
  const NON_ALPHANUMERIC_REGEXP = /([^#-~| |!])/g;
  const alphaFix = (value: string) => `&#${value.charCodeAt(0)};`;

  return value
    .replace(/&/g, '&amp;')
    .replace(SURROGATE_PAIR_REGEXP, pairFix)
    .replace(NON_ALPHANUMERIC_REGEXP, alphaFix)
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

export function generateTempId() {
  const s4 = () =>
    Math.floor((1 + Math.random()) * 0x10000)
      .toString(16)
      .substring(1);
  return `local-${s4()}${s4()}-${s4()}`;
}

export function generateContentId() {
  const s4 = () =>
    Math.floor((1 + Math.random()) * 0x10000)
      .toString(16)
      .substring(1);
  return `mcid-${s4()}${s4()}-${s4()}`;
}

export function isTempId(id: string) {
  if (!id || !_.isString(id)) {
    return false;
  }
  return id.slice(0, 6) === 'local-';
}

let ResourcePath: string | null = null;
let _imageData: string | null = null;
// resourcePath -> image basename -> path of the image
let _imageCache: { [resourcePath: string]: { [basename: string]: string } } = {};

// Returns the image names under the current resource path.
function ensureImageCacheReady() {
  const resourcePath: string = ResourcePath ?? AppEnv.getLoadSettings().resourcePath;
  ResourcePath = resourcePath;

  if (!_imageData) {
    const imageData: string = AppEnv.fileListCache().imageData || '{}';
    _imageData = imageData;
    _imageCache = JSON.parse(imageData) || {};
  }

  if (!_imageCache || !_imageCache[resourcePath]) {
    if (_imageCache == null) {
      _imageCache = {};
    }
    if (_imageCache[resourcePath] == null) {
      _imageCache[resourcePath] = {};
    }

    const imagesPath = path.join(resourcePath, 'static', 'images');
    const files = (fs.readdirSync(imagesPath, { recursive: true }) as string[]).map((f) =>
      path.join(imagesPath, f)
    );
    for (let file of files) {
      // On Windows, we get paths like C:\images\compose.png, but
      // Chromium doesn't accept the backward slashes. Convert to
      // C:/images/compose.png
      file = file.replace(/\\/g, '/');

      // On all platforms, characters like "#" and "?" are valid in filepaths
      // but invalid in URLs. When possible, switch back to paths relative to
      // the static dir to avoid chars in the path that could cause images not
      // to load. Not: does not use path.join because we remove trailing slash.
      file = file.replace(`${resourcePath}/static/`, '');

      const basename = path.basename(file);
      _imageCache[resourcePath][basename] = file;
    }
    AppEnv.fileListCache().imageData = JSON.stringify(_imageCache);
  }
  return _imageCache[resourcePath];
}

export function imageNamed(fullname: string) {
  const [name, ext] = fullname.split('.');

  const images = ensureImageCacheReady();

  const plat = process.platform != null ? process.platform : '';
  const ratio = window.devicePixelRatio != null ? window.devicePixelRatio : 1;

  let attempt = `${name}-${plat}@${ratio}x.${ext}`;
  if (images[attempt]) {
    return images[attempt];
  }
  attempt = `${name}@${ratio}x.${ext}`;
  if (images[attempt]) {
    return images[attempt];
  }
  attempt = `${name}-${plat}.${ext}`;
  if (images[attempt]) {
    return images[attempt];
  }
  attempt = `${name}.${ext}`;
  if (images[attempt]) {
    return images[attempt];
  }
  attempt = `${name}-${plat}@2x.${ext}`;
  if (images[attempt]) {
    return images[attempt];
  }
  attempt = `${name}@2x.${ext}`;
  if (images[attempt]) {
    return images[attempt];
  }
  attempt = `${name}-${plat}@1x.${ext}`;
  if (images[attempt]) {
    return images[attempt];
  }
  attempt = `${name}@1x.${ext}`;
  if (images[attempt]) {
    return images[attempt];
  }
  return null;
}

export function subjectWithPrefix(subject: string, prefix: string) {
  if (subject.search(/fwd:/i) === 0) {
    return subject.replace(/fwd:/i, prefix);
  } else if (subject.search(/re:/i) === 0) {
    return subject.replace(/re:/i, prefix);
  } else {
    return `${prefix} ${subject}`;
  }
}

// True of all arguments have the same domains
export function emailsHaveSameDomain(...args: (string | undefined)[]) {
  if (args.length < 2) {
    return false;
  }
  const domains = args.map((email = '') => {
    const parts = email.toLowerCase().trim().split('@');
    return parts[parts.length - 1];
  });
  const toMatch = domains[0];
  return _.every(domains, (domain) => domain.length > 0 && toMatch === domain);
}

export function emailHasCommonDomain(email = '') {
  const parts = email.toLowerCase().trim().split('@');
  const domain = parts[parts.length - 1];
  return commonDomains[domain] != null ? commonDomains[domain] : false;
}

// This looks for and removes plus-ing, it taks a VERY liberal approach
// to match an email address. We'd rather let false positives through.
export function toEquivalentEmailForm(email: string) {
  // https://regex101.com/r/iS7kD5/3
  // eslint-disable-next-line
  const [ignored, user, domain] = /^([^+]+).*@(.+)$/gi.exec(email) || [null, '', ''];
  return `${user}@${domain}`.trim().toLowerCase();
}

export function emailIsEquivalent(
  email1: string | null | undefined,
  email2: string | null | undefined
) {
  if (email1 == null) {
    email1 = '';
  }
  if (email2 == null) {
    email2 = '';
  }
  email1 = email1.toLowerCase().trim();
  email2 = email2.toLowerCase().trim();
  if (email1 === email2) {
    return true;
  }
  email1 = toEquivalentEmailForm(email1);
  email2 = toEquivalentEmailForm(email2);
  return email1 === email2;
}

export function rectVisibleInRect(
  r1: { left: number; right: number; top: number; bottom: number },
  r2: { left: number; right: number; top: number; bottom: number }
) {
  return !(r2.left > r1.right || r2.right < r1.left || r2.top > r1.bottom || r2.bottom < r1.top);
}

export function isEqualReact(
  a: unknown,
  b: unknown,
  options: { ignoreKeys?: string[]; functionsAreEqual?: boolean } = {}
) {
  return isEqual(a, b, {
    functionsAreEqual: true,
    ignoreKeys: (options.ignoreKeys != null ? options.ignoreKeys : []).concat(['id']),
  });
}

// Customized version of Underscore 1.8.2's isEqual function
// You can pass the following options:
//   - functionsAreEqual: if true then all functions are equal
//   - keysToIgnore: an array of object keys to ignore checks on
//   - logWhenFalse: logs when isEqual returns false
export function isEqual(
  a: unknown,
  b: unknown,
  options: { functionsAreEqual?: boolean; logWhenFalse?: boolean; ignoreKeys?: string[] } = {}
) {
  const value = _isEqual(a, b, [], [], options);
  if (options.logWhenFalse) {
    if (value === false) {
      console.log('isEqual is false', a, b, options);
    }
    return value;
  }
  return value;
}

// Unwraps an underscore-wrapped object (`_(obj)`).
function unwrapped(value: unknown) {
  if (
    (typeof value === 'object' || typeof value === 'function') &&
    value !== null &&
    '_wrapped' in value &&
    value._wrapped != null
  ) {
    return value._wrapped;
  }
  return value;
}

export function _isEqual(
  a: unknown,
  b: unknown,
  aStack: unknown[] | null | undefined,
  bStack: unknown[] | null | undefined,
  options: { functionsAreEqual?: boolean; ignoreKeys?: string[] } = {}
) {
  // Identical objects are equal. `0 is -0`, but they aren't identical.
  // See the [Harmony `egal`
  // proposal](http://wiki.ecmascript.org/doku.php?id=harmony:egal).
  if (a === b) {
    // Same as `a !== 0 || 1 / a === 1 / b`: tells 0 from -0.
    return Object.is(a, b);
  }
  // A strict comparison is necessary because `null == undefined`.
  if (a === null || b === null) {
    return a === b;
  }
  // Unwrap any wrapped objects.
  a = unwrapped(a);
  b = unwrapped(b);

  if (options.functionsAreEqual) {
    if (_.isFunction(a) && _.isFunction(b)) {
      return true;
    }
  }

  // Compare `[[Class]]` names.
  const className = toString.call(a);
  if (className !== toString.call(b)) {
    return false;
  }
  switch (className) {
    // Strings, numbers, regular expressions, dates, and booleans are
    // compared by value.
    // RegExps are coerced to strings for comparison (Note: '' + /a/i is '/a/i')
    case '[object RegExp]':
    case '[object String]':
      // Primitives and their corresponding object wrappers are equivalent;
      // thus, `"5"` is equivalent to `new String("5")`.
      return `${a}` === `${b}`;
    case '[object Number]':
      // `NaN`s are equivalent, but non-reflexive.
      // Object(NaN) is equivalent to NaN
      // eslint-disable-next-line
      if (Number(a) !== Number(a)) {
        return Number(b) !== Number(b); // eslint-disable-line
      }
      // An `egal` comparison is performed for other numeric values.
      if (Number(a) === 0) {
        return 1 / Number(a) === 1 / Number(b);
      } else {
        return Number(a) === Number(b);
      }
    case '[object Date]':
    case '[object Boolean]':
      // Coerce dates and booleans to numeric primitive values. Dates are
      // compared by their millisecond representations. Note that invalid
      // dates with millisecond representations of `NaN` are not
      // equivalent.
      return Number(a) === Number(b);
    default:
  }

  const areArrays = className === '[object Array]';
  if (!areArrays) {
    if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) {
      return false;
    }

    // Objects with different constructors are not equivalent, but
    // `Object`s or `Array`s from different frames are.
    const aCtor = a.constructor;
    const bCtor = b.constructor;
    if (
      aCtor !== bCtor &&
      !(
        _.isFunction(aCtor) &&
        aCtor instanceof aCtor &&
        _.isFunction(bCtor) &&
        bCtor instanceof bCtor
      ) &&
      'constructor' in a &&
      'constructor' in b
    ) {
      return false;
    }
  }
  // Assume equality for cyclic structures. The algorithm for detecting cyclic
  // structures is adapted from ES 5.1 section 15.12.3, abstract operation `JO`.

  // Initializing stack of traversed objects.
  // It's done here since we only need them for objects and arrays comparison.
  const aSeen = aStack != null ? aStack : [];
  const bSeen = bStack != null ? bStack : [];
  let { length } = aSeen;
  while (length--) {
    // Linear search. Performance is inversely proportional to the number of
    // unique nested structures.
    if (aSeen[length] === a) {
      return bSeen[length] === b;
    }
  }

  // Add the first object to the stack of traversed objects.
  aSeen.push(a);
  bSeen.push(b);

  // Both are arrays or non-null objects by now (the class names match, and a
  // non-array is rejected above unless both are objects).
  const objA = a as Record<string | number, unknown>;
  const objB = b as Record<string | number, unknown>;

  // Recursively compare objects and arrays.
  if (areArrays) {
    // Compare array lengths to determine if a deep comparison is necessary.
    length = (a as unknown[]).length;
    if (length !== (b as unknown[]).length) {
      return false;
    }
    // Deep compare the contents, ignoring non-numeric properties.
    while (length--) {
      if (!_isEqual(objA[length], objB[length], aSeen, bSeen, options)) {
        return false;
      }
    }
  } else {
    // Deep compare objects.
    let key: string;
    const keys = Object.keys(objA);
    ({ length } = keys);
    // Ensure that both objects contain the same number of properties
    // before comparing deep equality.
    if (Object.keys(objB).length !== length) {
      return false;
    }
    const keysToIgnore: { [key: string]: boolean } = {};
    if (options.ignoreKeys && _.isArray(options.ignoreKeys)) {
      for (key of options.ignoreKeys) {
        keysToIgnore[key] = true;
      }
    }
    while (length--) {
      // Deep compare each member
      key = keys[length];
      if (key in keysToIgnore) {
        continue;
      }
      if (!(_.has(objB, key) && _isEqual(objA[key], objB[key], aSeen, bSeen, options))) {
        return false;
      }
    }
  }
  // Remove the first object from the stack of traversed objects.
  aSeen.pop();
  bSeen.pop();
  return true;
}

// https://github.com/mailcheck/mailcheck/wiki/list-of-popular-domains
// As a hash for instant lookup.
export const commonDomains: { [domain: string]: boolean } = {
  'aol.com': true,
  'att.net': true,
  'comcast.net': true,
  'facebook.com': true,
  'gmail.com': true,
  'gmx.com': true,
  'googlemail.com': true,
  'google.com': true,
  'hotmail.com': true,
  'hotmail.co.uk': true,
  'mac.com': true,
  'me.com': true,
  'mail.com': true,
  'msn.com': true,
  'live.com': true,
  'sbcglobal.net': true,
  'verizon.net': true,
  'yahoo.com': true,
  'yahoo.co.uk': true,
  'email.com': true,
  'games.com': true,
  'gmx.net': true,
  'hush.com': true,
  'hushmail.com': true,
  'inbox.com': true,
  'lavabit.com': true,
  'love.com': true,
  'pobox.com': true,
  'rocketmail.com': true,
  'safe-mail.net': true,
  'wow.com': true,
  'ygm.com': true,
  'ymail.com': true,
  'zoho.com': true,
  'fastmail.fm': true,
  'bellsouth.net': true,
  'charter.net': true,
  'cox.net': true,
  'earthlink.net': true,
  'juno.com': true,
  'btinternet.com': true,
  'virginmedia.com': true,
  'blueyonder.co.uk': true,
  'freeserve.co.uk': true,
  'live.co.uk': true,
  'ntlworld.com': true,
  'o2.co.uk': true,
  'orange.net': true,
  'sky.com': true,
  'talktalk.co.uk': true,
  'tiscali.co.uk': true,
  'virgin.net': true,
  'wanadoo.co.uk': true,
  'bt.com': true,
  'sina.com': true,
  'qq.com': true,
  'naver.com': true,
  'hanmail.net': true,
  'daum.net': true,
  'nate.com': true,
  'yahoo.co.jp': true,
  'yahoo.co.kr': true,
  'yahoo.co.id': true,
  'yahoo.co.in': true,
  'yahoo.com.sg': true,
  'yahoo.com.ph': true,
  'hotmail.fr': true,
  'live.fr': true,
  'laposte.net': true,
  'yahoo.fr': true,
  'wanadoo.fr': true,
  'orange.fr': true,
  'gmx.fr': true,
  'sfr.fr': true,
  'neuf.fr': true,
  'free.fr': true,
  'gmx.de': true,
  'hotmail.de': true,
  'live.de': true,
  'online.de': true,
  't-online.de': true,
  'web.de': true,
  'yahoo.de': true,
  'mail.ru': true,
  'rambler.ru': true,
  'yandex.ru': true,
  'hotmail.be': true,
  'live.be': true,
  'skynet.be': true,
  'voo.be': true,
  'tvcablenet.be': true,
  'hotmail.com.ar': true,
  'live.com.ar': true,
  'yahoo.com.ar': true,
  'fibertel.com.ar': true,
  'speedy.com.ar': true,
  'arnet.com.ar': true,
  'yahoo.com.mx': true,
  'live.com.mx': true,
  'hotmail.es': true,
  'hotmail.com.mx': true,
  'prodigy.net.mx': true,
};

export const commonlyCapitalizedSalutations = [
  'grandpa',
  'grandfather',
  'gramps',
  'grampa',
  'grandaddy',
  'grandad',
  'granda',
  'grandma',
  'grandmother',
  'grandson',
  'granddaughter',
  'grandchild',
  'grandchildren',
  'appa',
  'pop',
  'papa',
  'tata',
  'issi',
  'anna',
  'amma',
  'nana',
  'granny',
  'grandmom',
  'nan',
  'nanny',
  'memaw',
  'aunt',
  'uncle',
  'aunts',
  'uncles',
  'ma',
  'mom',
  'mother',
  'dad',
  'father',
  'pa',
  'bud',
  'buds',
  'kid',
  'kids',
  'niece',
  'sister',
  'brother',
  'brothers',
  'nephew',
  'nephews',
  "y'all",
  'yall',
  'yinz',
  'yinzers',
  'cousin',
  'cousins',
  'parents',
  'man',
  'men',
  'dude',
  'bro',
  'buddy',
  'women',
  'girl',
  'girls',
  'son',
  'sons',
  'guy',
  'guys',
  'lady',
  'ladies',
];

export function hueForString(str: string | null) {
  if (!str) {
    str = '';
  }
  return (
    str
      .split('')
      .map((c) => c.charCodeAt(0))
      .reduce((n, a) => n + a, 0) % 360
  );
}

// Patterns of addresses that are likely not a person (likelyNonHumanEmail).
const NonHumanAt = '[-@+=]';
export const NonHumanEmailTerms = [
  'no[-_]?reply',
  'do[-_]?not[-_]?reply',
  `bounce[s]?${NonHumanAt}`,
  'postmaster',
  `notification[s]?${NonHumanAt}`,
  `jobs${NonHumanAt}`,
  `developer${NonHumanAt}`,
  `receipts${NonHumanAt}`,
  `support${NonHumanAt}`,
  `billing${NonHumanAt}`,
  `ebill${NonHumanAt}`,
  `hello${NonHumanAt}`,
  `customercare${NonHumanAt}`,
  `contact${NonHumanAt}`,
  `team${NonHumanAt}`,
  `status${NonHumanAt}`,
  `alert[s]?${NonHumanAt}`,
  'notify',
  'auto[-_]confirm',
  'invitations',
  'newsletter',
  `[-_]tracking${NonHumanAt}`,
  'reply[-_]',
  'room[-_]',
  `[-_]reply${NonHumanAt}`,
  `email${NonHumanAt}`,
  `welcome${NonHumanAt}`,
  `news${NonHumanAt}`,
  `info${NonHumanAt}`,
  `automated${NonHumanAt}`,
  `list[s]?${NonHumanAt}`,
  `distribute[s]?${NonHumanAt}`,
  `catchall${NonHumanAt}`,
  `catch[-_]all${NonHumanAt}`,
];

// Emails that nave no-reply or similar phrases in them are likely not a
// human. As such it's not worth the cost to do a lookup on that person.
//
// Also emails that are really long are likely computer-generated email
// strings used for bcc-based automated teasks.
export function likelyNonHumanEmail(email: string) {
  // simple catch for long emails that are almost always autoreplies
  if (email.length > 48) {
    return true;
  }

  // simple catch for things like hex sequences in prefixes
  const digitCount = email.split('@')[0].split(/[0-9]/g).length - 1;
  if (digitCount >= 6) {
    return true;
  }

  // more advanced scan for common patterns
  const terms = NonHumanEmailTerms;
  const reStr = `(${terms.join('|')})`;
  const re = new RegExp(reStr, 'gi');
  return re.test(email);
}

// Does the several tests you need to determine if a test range is within
// a bounds. Expects both objects to have `start` and `end` keys.
// Compares any values with <= and >=.
export function overlapsBounds(
  bounds: { start: number; end: number },
  test: { start: number; end: number }
) {
  return test.start <= bounds.end && test.end >= bounds.start;
}
