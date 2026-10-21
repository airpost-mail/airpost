/* eslint-disable no-var */
/* eslint-disable no-constant-condition */
/* eslint-disable no-prototype-builtins */
/*
 * decaffeinate suggestions:
 * DS104: Avoid inline assignments
 * DS205: Consider reworking code to avoid use of IIFEs
 * DS206: Consider reworking classes to avoid initClass
 * DS207: Consider shorter variations of null checks
 * Full docs: https://github.com/decaffeinate/decaffeinate/blob/master/docs/suggestions.md
 */
let errorLogger: { reportError(error: Error): void };
import _ from 'underscore';
import { valueForKeyPath, setValueForKeyPath, remove } from './utils';
import { Disposable, Emitter } from 'event-kit';
import { ConfigChannel, ConfigReply } from './channels';

if (process.type === 'renderer') {
  ({ errorLogger } = AppEnv);
} else {
  ({ errorLogger } = global);
}

// Where config.json lives: the main process's ConfigPersistenceManager, called directly in main
// and over the synchronous ConfigChannel from a window.
interface Persistence {
  getRawValuesString(): string;
  setRawValue(keyPath: string, value: unknown): void;
  setSettings(settings: ConfigObject): void;
}

function sendSync(channel: string, ...args: unknown[]): string | undefined {
  const { ipcRenderer } = require('electron');
  const reply = ipcRenderer.sendSync(channel, ...args) as ConfigReply;
  if (!reply || reply.error !== undefined) {
    throw new Error((reply && reply.error) || `no answer on ${channel}`);
  }
  return reply.value;
}

function persistence(): Persistence {
  if (process.type === 'renderer') {
    return {
      getRawValuesString: () => {
        const value = sendSync(ConfigChannel.GetRaw);
        if (value === undefined) {
          throw new Error(`no settings on ${ConfigChannel.GetRaw}`);
        }
        return value;
      },
      setRawValue: (keyPath, value) => {
        sendSync(ConfigChannel.SetRaw, keyPath, value);
      },
      setSettings: (settings) => {
        sendSync(ConfigChannel.SetAll, settings);
      },
    };
  }
  const manager = global.application.configPersistenceManager;
  return {
    getRawValuesString: () => manager.getRawValuesString(),
    setRawValue: (keyPath, value) => manager.setRawValue(keyPath, value, null),
    setSettings: (settings) => manager.setSettings(settings, null),
  };
}

// A JSON-schema subset describing one config key (or a tree of them under `properties`).
export interface ConfigSchema {
  type?: string | string[];
  default?: unknown;
  properties?: { [key: string]: ConfigSchema };
  items?: ConfigSchema;
  minimum?: unknown;
  maximum?: unknown;
  enum?: unknown;
  title?: string;
  description?: string;
  [key: string]: unknown;
}

type ConfigObject = { [key: string]: unknown };

// Config values are untyped JSON, so listeners get them as `any` (the public plugin contract).
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type ConfigChangeCallback = (event: { oldValue: any; newValue: any }) => void;

// Coerces `value` to the schema or throws; called with `this` = the Config class.
export type SchemaEnforcer = (
  this: typeof Config,
  keyPath: string | null,
  value: unknown,
  schema: ConfigSchema
) => unknown;

// Essential: Used to access all of N1's configuration details.
//
// An instance of this class is always available as the `AppEnv.config` global.
//
// ## Getting and setting config settings.
//
// ```coffee
// # Note that with no value set, ::get returns the setting's default value.
// AppEnv.config.get('my-package.myKey') # -> 'defaultValue'
//
// AppEnv.config.set('my-package.myKey', 'value')
// AppEnv.config.get('my-package.myKey') # -> 'value'
// ```
//
// You may want to watch for changes. Use {::observe} to catch changes to the setting.
//
// ```coffee
// AppEnv.config.set('my-package.myKey', 'value')
// AppEnv.config.observe 'my-package.myKey', (newValue) ->
//   # `observe` calls immediately and every time the value is changed
//   console.log 'My configuration changed:', newValue
// ```
//
// If you want a notification only when the value changes, use {::onDidChange}.
//
// ```coffee
// AppEnv.config.onDidChange 'my-package.myKey', ({newValue, oldValue}) ->
//   console.log 'My configuration changed:', newValue, oldValue
// ```
//
// ### Value Coercion
//
// Config settings each have a type specified by way of a
// [schema](json-schema.org). For example we might an integer setting that only
// allows integers greater than `0`:
//
// ```coffee
// # When no value has been set, `::get` returns the setting's default value
// AppEnv.config.get('my-package.anInt') # -> 12
//
// # The string will be coerced to the integer 123
// AppEnv.config.set('my-package.anInt', '123')
// AppEnv.config.get('my-package.anInt') # -> 123
//
// # The string will be coerced to an integer, but it must be greater than 0, so is set to 1
// AppEnv.config.set('my-package.anInt', '-20')
// AppEnv.config.get('my-package.anInt') # -> 1
// ```
//
// ## Defining settings for your package
//
// Define a schema under a `config` key in your package main.
//
// ```coffee
// module.exports =
//   # Your config schema
//   config:
//     someInt:
//       type: 'integer'
//       default: 23
//       minimum: 1
//
//   activate: (state) -> # ...
//   # ...
// ```
//
// ## Config Schemas
//
// We use [json schema](http://json-schema.org) which allows you to define your value's
// default, the type it should be, etc. A simple example:
//
// ```coffee
// # We want to provide an `enableThing`, and a `thingVolume`
// config:
//   enableThing:
//     type: 'boolean'
//     default: false
//   thingVolume:
//     type: 'integer'
//     default: 5
//     minimum: 1
//     maximum: 11
// ```
//
// The type keyword allows for type coercion and validation. If a `thingVolume` is
// set to a string `'10'`, it will be coerced into an integer.
//
// ```coffee
// AppEnv.config.set('my-package.thingVolume', '10')
// AppEnv.config.get('my-package.thingVolume') # -> 10
//
// # It respects the min / max
// AppEnv.config.set('my-package.thingVolume', '400')
// AppEnv.config.get('my-package.thingVolume') # -> 11
//
// # If it cannot be coerced, the value will not be set
// AppEnv.config.set('my-package.thingVolume', 'cats')
// AppEnv.config.get('my-package.thingVolume') # -> 11
// ```
//
// ### Supported Types
//
// The `type` keyword can be a string with any one of the following. You can also
// chain them by specifying multiple in an an array. For example
//
// ```coffee
// config:
//   someSetting:
//     type: ['boolean', 'integer']
//     default: 5
//
// # Then
// AppEnv.config.set('my-package.someSetting', 'true')
// AppEnv.config.get('my-package.someSetting') # -> true
//
// AppEnv.config.set('my-package.someSetting', '12')
// AppEnv.config.get('my-package.someSetting') # -> 12
// ```
//
// #### string
//
// Values must be a string.
//
// ```coffee
// config:
//   someSetting:
//     type: 'string'
//     default: 'hello'
// ```
//
// #### integer
//
// Values will be coerced into integer. Supports the (optional) `minimum` and
// `maximum` keys.
//
//   ```coffee
//   config:
//     someSetting:
//       type: 'integer'
//       default: 5
//       minimum: 1
//       maximum: 11
//   ```
//
// #### number
//
// Values will be coerced into a number, including real numbers. Supports the
// (optional) `minimum` and `maximum` keys.
//
// ```coffee
// config:
//   someSetting:
//     type: 'number'
//     default: 5.3
//     minimum: 1.5
//     maximum: 11.5
// ```
//
// #### boolean
//
// Values will be coerced into a Boolean. `'true'` and `'false'` will be coerced into
// a boolean. Numbers, arrays, objects, and anything else will not be coerced.
//
// ```coffee
// config:
//   someSetting:
//     type: 'boolean'
//     default: false
// ```
//
// #### array
//
// Value must be an Array. The types of the values can be specified by a
// subschema in the `items` key.
//
// ```coffee
// config:
//   someSetting:
//     type: 'array'
//     default: [1, 2, 3]
//     items:
//       type: 'integer'
//       minimum: 1.5
//       maximum: 11.5
// ```
//
// #### object
//
// Value must be an object. This allows you to nest config options. Sub options
// must be under a `properties key`
//
// ```coffee
// config:
//   someSetting:
//     type: 'object'
//     properties:
//       myChildIntOption:
//         type: 'integer'
//         minimum: 1.5
//         maximum: 11.5
// ```
//
// ### Other Supported Keys
//
// #### enum
//
// All types support an `enum` key. The enum key lets you specify all values
// that the config setting can possibly be. `enum` _must_ be an array of values
// of your specified type. Schema:
//
// ```coffee
// config:
//   someSetting:
//     type: 'integer'
//     default: 4
//     enum: [2, 4, 6, 8]
// ```
//
// Usage:
//
// ```coffee
// AppEnv.config.set('my-package.someSetting', '2')
// AppEnv.config.get('my-package.someSetting') # -> 2
//
// # will not set values outside of the enum values
// AppEnv.config.set('my-package.someSetting', '3')
// AppEnv.config.get('my-package.someSetting') # -> 2
//
// # If it cannot be coerced, the value will not be set
// AppEnv.config.set('my-package.someSetting', '4')
// AppEnv.config.get('my-package.someSetting') # -> 4
// ```
//
// #### title and description
//
// The settings view will use the `title` and `description` keys to display your
// config setting in a readable way. By default the settings view humanizes your
// config key, so `someSetting` becomes `Some Setting`. In some cases, this is
// confusing for users, and a more descriptive title is useful.
//
// Descriptions will be displayed below the title in the settings view.
//
// ```coffee
// config:
//   someSetting:
//     title: 'Setting Magnitude'
//     description: 'This will affect the blah and the other blah'
//     type: 'integer'
//     default: 4
// ```
//
// __Note__: You should strive to be so clear in your naming of the setting that
// you do not need to specify a title or description!
//
// ## Best practices
//
// * Don't depend on (or write to) configuration keys outside of your keypath.
//
export default class Config {
  static schemaEnforcers: { [typeName: string]: SchemaEnforcer[] } = {};

  static addSchemaEnforcer(typeName: string, enforcerFunction: SchemaEnforcer) {
    if (this.schemaEnforcers[typeName] == null) {
      this.schemaEnforcers[typeName] = [];
    }
    this.schemaEnforcers[typeName].push(enforcerFunction);
  }

  static addSchemaEnforcers(filters: { [typeName: string]: { [name: string]: SchemaEnforcer } }) {
    for (const typeName in filters) {
      const functions = filters[typeName];
      for (const name in functions) {
        const enforcerFunction = functions[name];
        this.addSchemaEnforcer(typeName, enforcerFunction);
      }
    }
  }

  static executeSchemaEnforcers(keyPath: string | null, value: unknown, schema: ConfigSchema) {
    let error: unknown = null;
    const types = Array.isArray(schema.type) ? schema.type : [schema.type];
    for (const type of types) {
      try {
        // An unknown (or missing) type has no enforcers: the TypeError here counts as a failure.
        const enforcerFunctions = this.schemaEnforcers[String(type)].concat(
          this.schemaEnforcers['*']
        );
        for (const enforcer of enforcerFunctions) {
          // At some point in one's life, one must call upon an enforcer.
          value = enforcer.call(this, keyPath, value, schema);
        }
        error = null;
        break;
      } catch (e) {
        error = e;
      }
    }

    if (error != null) {
      throw error;
    }
    return value;
  }

  settings: ConfigObject = {};
  defaultSettings: ConfigObject = {};
  transactDepth = 0;
  emitter = new Emitter();
  schema: ConfigSchema = {
    type: 'object',
    properties: {},
  };

  // Created during initialization, available as `AppEnv.config`
  constructor() {
    if (process.type === 'renderer') {
      const { ipcRenderer } = require('electron');
      // If new Config() has already been called, unmount it's listener when
      // we attach ourselves. This is only done during specs, Config is a singleton.
      ipcRenderer.removeAllListeners(ConfigChannel.Reloaded);
      ipcRenderer.on(ConfigChannel.Reloaded, (event: unknown, settings: ConfigObject) => {
        this.updateSettings(settings);
      });
    }
  }

  _logError(prefix: string, thrown: unknown) {
    const error = thrown instanceof Error ? thrown : new Error(String(thrown));
    error.message = `${prefix}: ${error.message}`;
    console.error(error.message);
    errorLogger.reportError(error);
  }

  load() {
    this.updateSettings(this.getRawValues());
  }

  /*
    Section: Config Subscription
    */

  // Essential: Add a listener for changes to a given key path. This is different
  // than {::onDidChange} in that it will immediately call your callback with the
  // current value of the config entry.
  //
  // ### Examples
  //
  // You might want to be notified when the themes change. We'll watch
  // `core.themes` for changes
  //
  // ```coffee
  // AppEnv.config.observe 'core.themes', (value) ->
  //   # do stuff with value
  // ```
  //
  // * `keyPath` {String} name of the key to observe
  // * `callback` {Function} to call when the value of the key changes.
  //   * `value` the new value of the key
  //
  // Returns a {Disposable} with the following keys on which you can call
  // `.dispose()` to unsubscribe.
  observe(keyPath: string, callback: (value: unknown) => void) {
    callback(this.get(keyPath));
    return this.onDidChangeKeyPath(keyPath, (event) => callback(event.newValue));
  }

  // Essential: Add a listener for changes to a given key path. If `keyPath` is
  // not specified, your callback will be called on changes to any key.
  //
  // * `keyPath` (optional) {String} name of the key to observe.
  // * `callback` {Function} to call when the value of the key changes.
  //   * `event` {Object}
  //     * `newValue` the new value of the key
  //     * `oldValue` the prior value of the key.
  //     * `keyPath` the keyPath of the changed key
  //
  // Returns a {Disposable} with the following keys on which you can call
  // `.dispose()` to unsubscribe.
  onDidChange(callback: ConfigChangeCallback): Disposable;
  onDidChange(keyPath: string | null | undefined, callback: ConfigChangeCallback): Disposable;
  onDidChange(...args: [ConfigChangeCallback] | [string | null | undefined, ConfigChangeCallback]) {
    const [keyPath, callback] = args.length === 1 ? [undefined, args[0]] : args;
    return this.onDidChangeKeyPath(keyPath, callback);
  }

  /*
    Section: Managing Settings
    */

  // Essential: Retrieves the setting for the given key.
  //
  // ### Examples
  //
  // You might want to know what themes are enabled, so check `core.themes`
  //
  // ```coffee
  // AppEnv.config.get('core.themes')
  // ```
  //
  // * `keyPath` The {String} name of the key to retrieve.
  //
  // Returns the value from N1's default settings, the user's configuration
  // file in the type specified by the configuration schema.
  // Settings are untyped JSON; `any` is the public contract for core and plugins.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  get(keyPath?: string | null): any {
    return this.getRawValue(keyPath);
  }

  // Essential: Sets the value for a configuration setting.
  //
  // This value is stored in N1's internal configuration file.
  //
  // ### Examples
  //
  // You might want to change the themes programmatically:
  //
  // ```coffee
  // AppEnv.config.set('core.themes', ['ui-light', 'my-custom-theme'])
  // ```
  //
  // * `keyPath` The {String} name of the key.
  // * `value` The value of the setting. Passing `undefined` will revert the
  //   setting to the default value.
  //
  // Returns a {Boolean}
  // * `true` if the value was set.
  // * `false` if the value was not able to be coerced to the type specified in the setting's schema.
  set(keyPath: string, value: unknown) {
    if (value === undefined) {
      value = valueForKeyPath(this.defaultSettings, keyPath);
    } else {
      try {
        value = this.makeValueConformToSchema(keyPath, value);
      } catch (e) {
        this._logError(`Failed to set keyPath: ${keyPath} = ${value}`, e);
        return false;
      }
    }

    // Only plain objects cross the IPC channel to the main process: no function bodies, no
    // class instances.
    if (_.isObject(value)) {
      value = JSON.parse(JSON.stringify(value));
    }

    this.setRawValue(keyPath, value);
    return true;
  }

  // Essential: Restore the setting at `keyPath` to its default value.
  //
  // * `keyPath` The {String} name of the key.
  // * `options` (optional) {Object}
  unset(keyPath: string) {
    this.set(keyPath, valueForKeyPath(this.defaultSettings, keyPath));
  }

  // Extended: Retrieve the schema for a specific key path. The schema will tell
  // you what type the keyPath expects, and other metadata about the config
  // option.
  //
  // * `keyPath` The {String} name of the key.
  //
  // Returns an {Object} eg. `{type: 'integer', default: 23, minimum: 1}`.
  // Returns `null` when the keyPath has no schema specified.
  getSchema(keyPath: string | null) {
    const keys = splitKeyPath(keyPath);
    let schema: ConfigSchema | undefined = this.schema;
    for (const key of keys) {
      if (schema == null) {
        break;
      }
      schema = schema.properties != null ? schema.properties[key] : undefined;
    }
    return schema;
  }

  // Extended: Suppress calls to handler functions registered with {::onDidChange}
  // and {::observe} for the duration of `callback`. After `callback` executes,
  // handlers will be called once if the value for their key-path has changed.
  //
  // * `callback` {Function} to execute while suppressing calls to handlers.
  transact<T>(callback: () => T): T {
    this.transactDepth++;
    try {
      return callback();
    } finally {
      this.transactDepth--;
      this.emitChangeEvent();
    }
  }

  /*
    Section: Internal methods used by core
    */

  pushAtKeyPath(keyPath: string, value: unknown) {
    const arrayValue = this.get(keyPath) || [];
    if (!(arrayValue instanceof Array)) {
      throw new Error(
        `Config.pushAtKeyPath is intended for array values. Value ${JSON.stringify(
          arrayValue
        )} is not an array.`
      );
    }
    const result = arrayValue.push(value);
    this.set(keyPath, arrayValue);
    return result;
  }

  unshiftAtKeyPath(keyPath: string, value: unknown) {
    const arrayValue = this.get(keyPath) || [];
    if (!(arrayValue instanceof Array)) {
      throw new Error(
        `Config.unshiftAtKeyPath is intended for array values. Value ${JSON.stringify(
          arrayValue
        )} is not an array.`
      );
    }
    const result = arrayValue.unshift(value);
    this.set(keyPath, arrayValue);
    return result;
  }

  removeAtKeyPath(keyPath: string, value: unknown) {
    const arrayValue = this.get(keyPath) || [];
    if (!(arrayValue instanceof Array)) {
      throw new Error(
        `Config.removeAtKeyPath is intended for array values. Value ${JSON.stringify(
          arrayValue
        )} is not an array.`
      );
    }
    const result = remove(arrayValue, value);
    this.set(keyPath, arrayValue);
    return result;
  }

  // A null keyPath sets the root schema.
  setSchema(keyPath: string | null, schema: ConfigSchema) {
    if (!isPlainObject(schema)) {
      throw new Error(`Error loading schema for ${keyPath}: schemas can only be objects!`);
    }

    if (!typeof (schema.type != null)) {
      throw new Error(
        `Error loading schema for ${keyPath}: schema objects must have a type attribute`
      );
    }

    let rootSchema = this.schema;
    if (keyPath) {
      for (const key of splitKeyPath(keyPath)) {
        rootSchema.type = 'object';
        if (rootSchema.properties == null) {
          rootSchema.properties = {};
        }
        const { properties } = rootSchema;
        if (properties[key] == null) {
          properties[key] = {};
        }
        rootSchema = properties[key];
      }
    }

    Object.assign(rootSchema, schema);
    this.setDefaults(keyPath, this.extractDefaultsFromSchema(schema));
    this.resetSettingsForSchemaChange();
  }

  /*
    Section: Private methods managing global settings
    */

  updateSettings = (newSettings: ConfigObject) => {
    if (!newSettings || _.isEmpty(newSettings)) {
      throw new Error(`Tried to update settings with false-y value: ${newSettings}`);
    }
    this.settings = newSettings;
    this.emitChangeEvent();
  };

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  getRawValue(keyPath?: string | null): any {
    let value = valueForKeyPath(this.settings, keyPath);
    const defaultValue = valueForKeyPath(this.defaultSettings, keyPath);

    if (value != null) {
      value = this.deepClone(value);
      if (isPlainObject(value) && isPlainObject(defaultValue)) {
        _.defaults(value, defaultValue);
      }
    } else {
      value = this.deepClone(defaultValue);
    }

    return value;
  }

  onDidChangeKeyPath(
    keyPath: string | null | undefined,
    callback: (event: { oldValue: unknown; newValue: unknown }) => void
  ) {
    let oldValue = this.get(keyPath);
    return this.emitter.on('did-change', () => {
      const newValue = this.get(keyPath);
      if (!_.isEqual(oldValue, newValue)) {
        const event = { oldValue, newValue };
        oldValue = newValue;
        callback(event);
      }
    });
  }

  isSubKeyPath(keyPath?: string, subKeyPath?: string) {
    if (keyPath == null || subKeyPath == null) {
      return false;
    }
    const pathSubTokens = splitKeyPath(subKeyPath);
    const pathTokens = splitKeyPath(keyPath).slice(0, pathSubTokens.length);
    return _.isEqual(pathTokens, pathSubTokens);
  }

  setRawDefault(keyPath: string | null, value: unknown) {
    setValueForKeyPath(this.defaultSettings, keyPath, value);
    this.emitChangeEvent();
  }

  setDefaults(keyPath: string | null, defaults: unknown) {
    if (defaults != null && isPlainObject(defaults)) {
      const keys = splitKeyPath(keyPath);
      for (const key in defaults) {
        const childValue = defaults[key];
        if (!defaults.hasOwnProperty(key)) {
          continue;
        }
        this.setDefaults(keys.concat([key]).join('.'), childValue);
      }
    } else {
      try {
        defaults = this.makeValueConformToSchema(keyPath, defaults);
        this.setRawDefault(keyPath, defaults);
      } catch (e) {
        this._logError(
          `Failed to set keypath to default: ${keyPath} = ${JSON.stringify(defaults)}`,
          e
        );
      }
    }
  }

  deepClone(object: unknown): unknown {
    if (Array.isArray(object)) {
      return object.map((value): unknown => this.deepClone(value));
    } else if (isPlainObject(object)) {
      return _.mapObject(object, (value): unknown => this.deepClone(value));
    } else {
      return object;
    }
  }

  extractDefaultsFromSchema(schema: ConfigSchema): unknown {
    if (schema.default != null) {
      return schema.default;
    } else if (
      schema.type === 'object' &&
      schema.properties != null &&
      isPlainObject(schema.properties)
    ) {
      const defaults: ConfigObject = {};
      const properties = schema.properties || {};
      for (const key in properties) {
        const value = properties[key];
        defaults[key] = this.extractDefaultsFromSchema(value);
      }
      return defaults;
    }
  }

  makeValueConformToSchema(keyPath: string | null, value: unknown) {
    const schema = this.getSchema(keyPath);
    if (schema) {
      value = Config.executeSchemaEnforcers(keyPath, value, schema);
    }
    return value;
  }

  // When the schema is changed / added, there may be values set in the config
  // that do not conform to the schema. This reset will make them conform.
  resetSettingsForSchemaChange() {
    return this.transact(() => {
      let settings = this.getRawValues();
      settings = this.makeValueConformToSchema(null, settings);
      if (!settings || _.isEmpty(settings)) {
        throw new Error('settings is falsey or empty');
      }
      persistence().setSettings(settings);
    });
  }

  emitChangeEvent() {
    if (!(this.transactDepth > 0)) {
      this.emitter.emit('did-change');
    }
  }

  getRawValues() {
    try {
      const result = JSON.parse(persistence().getRawValuesString());
      if (!result || _.isEmpty(result)) {
        throw new Error('settings is falsey or empty');
      }
      return result;
    } catch (error) {
      this._logError('Failed to parse response from getRawValuesString', error);
      throw error;
    }
  }

  setRawValue(keyPath: string, value: unknown) {
    persistence().setRawValue(keyPath, value);
    this.load();
  }
}

// Base schema enforcers. These will coerce raw input into the specified type,
// and will throw an error when the value cannot be coerced. Throwing the error
// will indicate that the value should not be set.
//
// Enforcers are run from most specific to least. For a schema with type
// `integer`, all the enforcers for the `integer` type will be run first, in
// order of specification. Then the `*` enforcers will be run, in order of
// specification.
Config.addSchemaEnforcers({
  integer: {
    coerce(keyPath, value, schema) {
      const int = parseInt(String(value), 10);
      if (isNaN(int) || !isFinite(int)) {
        throw new Error(
          `Validation failed at ${keyPath}, ${JSON.stringify(int)} cannot be coerced into an int`
        );
      }
      return int;
    },
  },

  number: {
    coerce(keyPath, value, schema) {
      const num = parseFloat(String(value));
      if (isNaN(num) || !isFinite(num)) {
        throw new Error(
          `Validation failed at ${keyPath}, ${JSON.stringify(num)} cannot be coerced into a number`
        );
      }
      return num;
    },
  },

  boolean: {
    coerce(keyPath, value, schema) {
      switch (typeof value) {
        case 'string':
          if (value.toLowerCase() === 'true') {
            return true;
          } else if (value.toLowerCase() === 'false') {
            return false;
          } else {
            throw new Error(
              `Validation failed at ${keyPath}, ${JSON.stringify(
                value
              )} must be a boolean or the string 'true' or 'false'`
            );
          }
        case 'boolean':
          return value;
        default:
          throw new Error(
            `Validation failed at ${keyPath}, ${JSON.stringify(
              value
            )} must be a boolean or the string 'true' or 'false'`
          );
      }
    },
  },

  string: {
    validate(keyPath, value, schema) {
      if (typeof value !== 'string') {
        throw new Error(
          `Validation failed at ${keyPath}, ${JSON.stringify(value)} must be a string`
        );
      }
      return value;
    },
  },

  null: {
    // null sort of isnt supported. It will just unset in this case
    coerce(keyPath, value, schema) {
      if (value !== undefined && value !== null) {
        throw new Error(`Validation failed at ${keyPath}, ${JSON.stringify(value)} must be null`);
      }
      return value;
    },
  },

  object: {
    coerce(keyPath, value, schema) {
      if (!isPlainObject(value)) {
        throw new Error(
          `Validation failed at ${keyPath}, ${JSON.stringify(value)} must be an object`
        );
      }
      if (schema.properties == null) {
        return value;
      }

      const newValue: ConfigObject = {};
      for (const prop in value) {
        const propValue = value[prop];
        const childSchema = schema.properties[prop];
        if (childSchema != null) {
          try {
            newValue[prop] = this.executeSchemaEnforcers(
              `${keyPath}.${prop}`,
              propValue,
              childSchema
            );
          } catch (error) {
            console.warn(`Error setting item in object: ${errorMessage(error)}`);
          }
        } else {
          // Just pass through un-schema'd values
          newValue[prop] = propValue;
        }
      }

      return newValue;
    },
  },

  array: {
    coerce(keyPath, value, schema) {
      if (!Array.isArray(value)) {
        throw new Error(
          `Validation failed at ${keyPath}, ${JSON.stringify(value)} must be an array`
        );
      }
      const itemSchema = schema.items;
      if (itemSchema != null) {
        const newValue: unknown[] = [];
        for (const item of value) {
          try {
            newValue.push(this.executeSchemaEnforcers(keyPath, item, itemSchema));
          } catch (error) {
            console.warn(`Error setting item in array: ${errorMessage(error)}`);
          }
        }
        return newValue;
      } else {
        return value;
      }
    },
  },

  '*': {
    coerceMinimumAndMaximum(keyPath, value, schema) {
      if (typeof value !== 'number') {
        return value;
      }
      let num = value;
      if (schema.minimum != null && typeof schema.minimum === 'number') {
        num = Math.max(num, schema.minimum);
      }
      if (schema.maximum != null && typeof schema.maximum === 'number') {
        num = Math.min(num, schema.maximum);
      }
      return num;
    },

    validateEnum(keyPath, value, schema) {
      const possibleValues = schema.enum;
      if (possibleValues == null || !Array.isArray(possibleValues) || !possibleValues.length) {
        return value;
      }

      for (const possibleValue of possibleValues) {
        // Using `isEqual` for possibility of placing enums on array and object schemas
        if (_.isEqual(possibleValue, value)) {
          return value;
        }
      }

      throw new Error(
        `Validation failed at ${keyPath}, ${JSON.stringify(value)} is not one of ${JSON.stringify(
          possibleValues
        )}`
      );
    },
  },
});

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function isPlainObject(value: unknown): value is ConfigObject {
  return _.isObject(value) && !_.isArray(value) && !_.isFunction(value) && !_.isString(value);
}

function splitKeyPath(keyPath: string | null | undefined): string[] {
  if (keyPath == null) {
    return [];
  }
  let startIndex = 0;
  const keyPathArray: string[] = [];
  for (let i = 0; i < keyPath.length; i++) {
    const char = keyPath[i];
    if (char === '.' && (i === 0 || keyPath[i - 1] !== '\\')) {
      keyPathArray.push(keyPath.substring(startIndex, i));
      startIndex = i + 1;
    }
  }
  keyPathArray.push(keyPath.substr(startIndex, keyPath.length));
  return keyPathArray;
}
