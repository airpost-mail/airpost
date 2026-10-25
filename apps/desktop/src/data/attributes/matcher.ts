import { QueryExpression } from '../../mail/search/ast';
import { Attribute } from './attribute';
import { AttributeCollection } from './collection';
import { Model } from '../models/model';
import { attributeValue, ComparableValue } from '../models/attribute-access';

// https://www.sqlite.org/faq.html#q14
// That's right. Two single quotes in a row…
const singleQuoteEscapeSequence = "''";

// https://www.sqlite.org/fts5.html#section_3
const doubleQuoteEscapeSequence = '""';

/** A matcher as `ModelQuery.toJSON()` sends it to the engine's `query`. */
export type MatcherJSON =
  | { attr: string; op: string; value: unknown }
  | { and: MatcherJSON[] }
  | { or: MatcherJSON[] }
  | { not: MatcherJSON[] };

/*
Public: The Matcher class encapsulates a particular comparison clause on an {Attribute}.
Matchers can evaluate whether or not an object matches them, and also compose
SQL clauses for the DatabaseStore. Each matcher has a reference to a model
attribute, a comparator and a value.

```javascript

// Retrieving Matchers

isUnread = Thread.attributes.unread.equal(true)

hasLabel = Thread.attributes.categories.contains('label-id-123')

// Using Matchers in Database Queries

DatabaseStore.findAll<Thread>(Thread).where(isUnread)...

// Using Matchers to test Models

threadA = new Thread(unread: true)
threadB = new Thread(unread: false)

isUnread.evaluate(threadA)
// => true
isUnread.evaluate(threadB)
// => false

```

Section: Database
*/
export class Matcher {
  static get Or() {
    return OrCompositeMatcher;
  }
  static get And() {
    return AndCompositeMatcher;
  }
  static get Not() {
    return NotCompositeMatcher;
  }
  static get Search() {
    return SearchMatcher;
  }
  static get StructuredSearch() {
    return StructuredSearchMatcher;
  }

  static muid = 0;

  // Only the leaf matchers made by Attribute have an attribute and a comparator; the
  // composite and search matchers leave them empty.
  attr?: Attribute | null;
  comparator?: string | null;
  val: any;
  muid: number;

  constructor(attr?: Attribute | null, comparator?: string | null, val?: any) {
    this.attr = attr;
    this.comparator = comparator;
    this.val = val;

    this.muid = Matcher.muid;
    Matcher.muid = (Matcher.muid + 1) % 50;
  }

  attribute(): Attribute | null | undefined {
    return this.attr;
  }

  value() {
    return this.val;
  }

  // The attribute and comparator of a leaf matcher. The composite and search matchers
  // override every method that reads them.
  protected leaf(): { attr: Attribute; comparator: string } {
    if (!this.attr || !this.comparator) {
      throw new Error(`Matcher (${this.comparator}) has no attribute`);
    }
    return { attr: this.attr, comparator: this.comparator };
  }

  protected leafAttr(): Attribute {
    return this.leaf().attr;
  }

  evaluate(model: Model) {
    const attr = this.leafAttr();
    let modelValue: unknown = attributeValue(model, attr.modelKey);
    if (modelValue instanceof Function) {
      modelValue = modelValue();
    }
    let matcherValue = this.val;

    // Dates are compared as milliseconds so `=` is not object identity and so a
    // numeric bound is read as the unix seconds the column stores, matching whereSQL.
    if (modelValue instanceof Date) {
      modelValue = modelValue.getTime();
      if (matcherValue instanceof Date) {
        matcherValue = matcherValue.getTime();
      } else if (typeof matcherValue === 'number') {
        matcherValue = matcherValue * 1000;
      }
    }

    const asId = (v: unknown) => (v && typeof v === 'object' && 'id' in v && v.id ? v.id : v);
    // The comparator says what the attribute holds: an ordered value, a list or a string.
    const ordered = modelValue as ComparableValue;

    switch (this.comparator) {
      case '=':
        // triple-equals would break this, because we convert false to 0, true to 1
        return modelValue == matcherValue; // eslint-disable-line
      case '!=':
        return modelValue != matcherValue; // eslint-disable-line
      case '<':
        return ordered < matcherValue;
      case '>':
        return ordered > matcherValue;
      case '<=':
        return ordered <= matcherValue;
      case '>=':
        return ordered >= matcherValue;
      case 'in':
        return matcherValue.includes(modelValue);
      case 'not in':
        return !matcherValue.includes(modelValue);
      case 'contains': {
        const search = asId(matcherValue);
        for (const item of modelValue as unknown[]) {
          if (asId(item) === search) return true;
        }
        return false;
      }
      case 'containsAny': {
        const idSet = new Set((modelValue as unknown[]).map(asId));
        return matcherValue.some((submatcherValue: unknown) => idSet.has(asId(submatcherValue)));
      }
      case 'startsWith':
        return (modelValue as string).startsWith(matcherValue);
      case 'like':
        return (modelValue as string).search(new RegExp(`.*${matcherValue}.*`, 'gi')) >= 0;
      default:
        throw new Error(
          `Matcher.evaulate() not sure how to evaluate ${attr.modelKey} with comparator ${this.comparator}`
        );
    }
  }

  // The JSON form the engine's `query` takes. Comparators
  // and values go as the SQL above uses them, so `AttributeBoolean.equal(true)` is sent as
  // `!= 0` and a Date as the unix seconds its column stores.
  toJSON(): MatcherJSON {
    const value = (v: unknown) => (v instanceof Date ? v.getTime() / 1000 : v);
    return {
      attr: this.leaf().attr.modelKey,
      op: this.leaf().comparator,
      value: this.val instanceof Array ? this.val.map(value) : value(this.val),
    };
  }

  joinTableRef() {
    return `M${this.muid}`;
  }

  joinSQL(klass: typeof Model): string | false {
    switch (this.comparator) {
      case 'contains':
      case 'containsAny': {
        const joinTable = (this.leafAttr() as AttributeCollection).tableNameForJoinAgainst(klass);
        const joinTableRef = this.joinTableRef();
        return `INNER JOIN \`${joinTable}\` AS \`${joinTableRef}\` ON \`${joinTableRef}\`.\`id\` = \`${klass.name}\`.\`id\``;
      }
      default:
        return false;
    }
  }

  whereSQL(klass: typeof Model): string {
    const attr = this.leafAttr();
    const val = this.comparator === 'like' ? `%${this.val}%` : this.val;
    let escaped: string | number | null = null;

    if (typeof val === 'string') {
      escaped = `'${val.replace(/'/g, singleQuoteEscapeSequence)}'`;
    } else if (val === true) {
      escaped = 1;
    } else if (val === false) {
      escaped = 0;
    } else if (val instanceof Date) {
      escaped = val.getTime() / 1000;
    } else if (val instanceof Array) {
      const escapedVals: string[] = [];
      for (const v of val) {
        if (typeof v !== 'string') {
          throw new Error(`${attr.tableColumn} value ${v} must be a string.`);
        }
        escapedVals.push(`'${v.replace(/'/g, singleQuoteEscapeSequence)}'`);
      }
      escaped = `(${escapedVals.join(',')})`;
    } else {
      escaped = val;
    }

    switch (this.comparator) {
      case '=': {
        if (escaped === null) {
          return `\`${klass.name}\`.\`${attr.tableColumn}\` IS NULL`;
        }
        return `\`${klass.name}\`.\`${attr.tableColumn}\` = ${escaped}`;
      }
      case '!=': {
        if (escaped === null) {
          return `\`${klass.name}\`.\`${attr.tableColumn}\` IS NOT NULL`;
        }
        return `\`${klass.name}\`.\`${attr.tableColumn}\` != ${escaped}`;
      }
      case 'startsWith': {
        const escapedVal =
          typeof val === 'string'
            ? val.replace(/'/g, singleQuoteEscapeSequence).replace(/[\\%_]/g, (c) => `\\${c}`)
            : val;
        return `\`${klass.name}\`.\`${attr.tableColumn}\` LIKE '${escapedVal}%' ESCAPE '\\'`;
      }
      case 'contains':
        return `\`${this.joinTableRef()}\`.\`value\` = ${escaped}`;
      case 'containsAny':
        return `\`${this.joinTableRef()}\`.\`value\` IN ${escaped}`;
      default:
        return `\`${klass.name}\`.\`${attr.tableColumn}\` ${this.comparator} ${escaped}`;
    }
  }
}

// Export the composite matchers for use as types
export class OrCompositeMatcher extends Matcher {
  children: Matcher[];

  constructor(children: Matcher[]) {
    super();
    this.children = children;
  }

  attribute() {
    return null;
  }

  value() {
    return null;
  }

  evaluate(model: Model) {
    return this.children.some((matcher) => matcher.evaluate(model));
  }

  toJSON(): MatcherJSON {
    return { or: this.children.map((m) => m.toJSON()) };
  }

  joinSQL(klass: typeof Model) {
    const joins: string[] = [];
    for (const matcher of this.children) {
      const join = matcher.joinSQL(klass);
      if (join) {
        joins.push(join);
      }
    }
    return joins.length ? joins.join(' ') : false;
  }

  whereSQL(klass: typeof Model) {
    const wheres = this.children.map((matcher) => matcher.whereSQL(klass));
    return `(${wheres.join(' OR ')})`;
  }
}

export class AndCompositeMatcher extends Matcher {
  children: Matcher[];

  constructor(children: Matcher[]) {
    super();
    this.children = children;
  }

  attribute() {
    return null;
  }

  value() {
    return null;
  }

  evaluate(model: Model) {
    return this.children.every((m) => m.evaluate(model));
  }

  toJSON(): MatcherJSON {
    return { and: this.children.map((m) => m.toJSON()) };
  }

  joinSQL(klass: typeof Model) {
    const joins: string[] = [];
    for (const matcher of this.children) {
      const join = matcher.joinSQL(klass);
      if (join) {
        joins.push(join);
      }
    }
    return joins.join(' ');
  }

  whereSQL(klass: typeof Model) {
    const wheres = this.children.map((m) => m.whereSQL(klass));
    return `(${wheres.join(' AND ')})`;
  }
}

class NotCompositeMatcher extends AndCompositeMatcher {
  toJSON(): MatcherJSON {
    return { not: this.children.map((m) => m.toJSON()) };
  }

  whereSQL(klass: typeof Model) {
    return `NOT (${super.whereSQL(klass)})`;
  }
}

// The search grammar, which the engine reads (`search {text, grammar: true}` in `query`), so
// queries with one always go to the engine. It takes the text, or an expression
// SearchQueryParser.parse made, which carries its text.
class StructuredSearchMatcher extends Matcher {
  text: string;
  // Relative dates (`after:yesterday`) count back from when the query was made.
  now: number;

  constructor(searchQuery: QueryExpression | string) {
    const text = typeof searchQuery === 'string' ? searchQuery : searchQuery.sourceText;
    if (!text) {
      throw new Error(
        'structuredSearch takes the search text, or what SearchQueryParser.parse made of it'
      );
    }
    super(null, null, null);
    this.text = text;
    this.now = Math.floor(Date.now() / 1000);
  }

  searchJSON() {
    let tz: string | undefined;
    try {
      tz = Intl.DateTimeFormat().resolvedOptions().timeZone || undefined;
    } catch (err) {
      tz = undefined;
    }
    return { text: this.text, grammar: true, tz, now: this.now };
  }

  attribute() {
    return null;
  }

  value() {
    return null;
  }

  // The only way to truly check if a model matches this matcher is to run the query
  // again and check if the model is in the results. This is too expensive, so we
  // will always return true so models aren't excluded from the
  // SearchQuerySubscription result set
  evaluate() {
    return true;
  }

  // ModelQuery.toJSON() sends it as the query's `search`.
  toJSON(): MatcherJSON {
    throw new Error("structuredSearch is the query's search, not a condition");
  }

  // What `sql()` compares queries by. Never run: DatabaseStore sends these queries to the
  // engine, and SQLite has no such function.
  whereSQL(klass: typeof Model): string {
    return `tern_search_grammar('${this.text.replace(/'/g, singleQuoteEscapeSequence)}')`;
  }
}

class SearchMatcher extends Matcher {
  searchQuery: string;
  /** The text as given; the engine applies the trimming and escaping below itself. */
  text: string;

  constructor(searchQuery: string) {
    if (typeof searchQuery !== 'string' || searchQuery.length === 0) {
      throw new Error('You must pass a string with non-zero length to search.');
    }
    super(null, null, null);
    this.text = searchQuery;
    this.searchQuery = searchQuery
      .trim()
      .replace(/^['"]/, '')
      .replace(/['"]$/, '')
      .replace(/'/g, singleQuoteEscapeSequence)
      .replace(/"/g, doubleQuoteEscapeSequence);
  }

  attribute() {
    return null;
  }

  value() {
    return null;
  }

  // The only way to truly check if a model matches this matcher is to run the query
  // again and check if the model is in the results. This is too expensive, so we
  // will always return true so models aren't excluded from the
  // SearchQuerySubscription result set
  evaluate() {
    return true;
  }

  // `ModelQuery.toJSON()` sends a top-level search as `search: {text, grammar: false}`.
  toJSON(): MatcherJSON {
    throw new Error('A search matcher is only valid at the top level of a query');
  }

  searchJSON() {
    return { text: this.text, grammar: false };
  }

  whereSQL(klass: typeof Model) {
    const searchTable = `${klass.name}Search`;
    return `\`${klass.name}\`.\`id\` IN (SELECT \`content_id\` FROM \`${searchTable}\` WHERE \`${searchTable}\` MATCH '"${this.searchQuery}"*' LIMIT 1000)`;
  }
}
