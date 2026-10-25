import { localized } from '../../intl/localize';
import { Message } from '../../data/models/message';
import { Observable } from 'rx-core';

// What the rule editor offers. The engine evaluates the rules (crates/tern-tasks mail_rules.rs);
// `fn` and `arrayMatchFn` describe the comparison for plugins that define their own.
export class Comparator {
  name: string;
  arrayMatchFn: any;
  fn: any;

  constructor(
    { name, arrayMatchFn }: { name: string; arrayMatchFn: typeof Array.prototype.some },
    fn: (args: { actual: any; desired: any }) => boolean
  ) {
    this.name = name;
    this.fn = fn;
    this.arrayMatchFn = arrayMatchFn;
  }
}

const Types = {
  None: 'None',
  Enum: 'Enum',
  String: 'String',
  InputString: 'InputString',
};

export const Comparators = {
  String: {
    contains: new Comparator(
      {
        name: localized('contains'),
        arrayMatchFn: Array.prototype.some,
      },
      ({ actual, desired }) => {
        if (!actual || !desired) {
          return false;
        }
        return actual.toLowerCase().includes(desired.toLowerCase());
      }
    ),

    doesNotContain: new Comparator(
      {
        name: localized('does not contain'),
        arrayMatchFn: Array.prototype.every,
      },
      ({ actual, desired }) => {
        if (!desired) {
          return false;
        }
        if (!actual) {
          return true;
        }
        return !actual.toLowerCase().includes(desired.toLowerCase());
      }
    ),

    beginsWith: new Comparator(
      {
        name: localized('begins with'),
        arrayMatchFn: Array.prototype.some,
      },
      ({ actual, desired }) => {
        if (!actual || !desired) {
          return false;
        }
        return actual.toLowerCase().startsWith(desired.toLowerCase());
      }
    ),

    endsWith: new Comparator(
      {
        name: localized('ends with'),
        arrayMatchFn: Array.prototype.some,
      },
      ({ actual, desired }) => {
        if (!actual || !desired) {
          return false;
        }
        return actual.toLowerCase().endsWith(desired.toLowerCase());
      }
    ),

    equals: new Comparator(
      {
        name: localized('equals'),
        arrayMatchFn: Array.prototype.some,
      },
      ({ actual, desired }) => {
        if (!actual || !desired) {
          return actual === desired;
        }
        return actual.toLowerCase() === desired.toLowerCase();
      }
    ),

    matchesExpression: new Comparator(
      {
        name: localized('matches expression'),
        arrayMatchFn: Array.prototype.some,
      },
      ({ actual, desired }) => {
        if (!actual || !desired) {
          return false;
        }
        return new RegExp(desired, 'gi').test(actual);
      }
    ),
  },
};

interface TemplateEnumValue {
  name: string;
  value: string;
}

// comparatorKey is cleared when the template has no comparators.
export interface TemplateInstance {
  templateKey: string;
  comparatorKey?: string;
  value: any;
}

const ComparatorsByType: { [type: string]: { [comparatorKey: string]: Comparator } | undefined } =
  Comparators;

export class Template {
  static Type = Types;
  static Comparator = Comparator;
  static Comparators = Comparators;

  key: string;
  // name and comparators are set by the Object.assign of defaults in the constructor.
  name!: string;
  type: string;
  values: Observable<TemplateEnumValue[]> | TemplateEnumValue[] | undefined;
  valueLabel: string | undefined;
  valueForMessage?: (message: Message) => any;
  comparators!: { [comparatorKey: string]: Comparator };

  constructor(key: string, type: string, options: Partial<Template> = {}) {
    this.key = key;
    this.type = type;

    const defaults = {
      name: this.key,
      values: undefined,
      valueLabel: undefined,
      comparators: ComparatorsByType[this.type] || {},
    };

    Object.assign(this, defaults, options);

    if (!this.key) {
      throw new Error('You must provide a valid key.');
    }
    if (!(this.type in Types)) {
      throw new Error('You must provide a valid type.');
    }
    if (this.type === Types.Enum && !this.values) {
      throw new Error('You must provide `values` when creating an enum.');
    }
  }

  createDefaultInstance() {
    return {
      templateKey: this.key,
      comparatorKey: Object.keys(this.comparators)[0],
      value: undefined,
    };
  }

  coerceInstance(instance: TemplateInstance): TemplateInstance {
    instance.templateKey = this.key;
    if (!this.comparators) {
      instance.comparatorKey = undefined;
    } else if (
      instance.comparatorKey === undefined ||
      !Object.keys(this.comparators).includes(instance.comparatorKey)
    ) {
      instance.comparatorKey = Object.keys(this.comparators)[0];
    }
    return instance;
  }
}
