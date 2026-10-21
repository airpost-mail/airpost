import { Attribute } from './attribute';
import { Matcher } from './matcher';
/*
Public: The value of this attribute is always a boolean. Null values are coerced to false.

String attributes can be queries using `equal` and `not`. Matching on
`greaterThan` and `lessThan` is not supported.

Section: Database
*/
export class AttributeBoolean extends Attribute {
  // Never set by the attribute definitions today; columnSQL reads it for the column default.
  defaultValue?: boolean;

  toJSON(val: unknown) {
    return val;
  }
  fromJSON(val: unknown) {
    // Some attributes we identify as booleans in Tern are ints
    // in the underlying sync engine for reference-counting purposes.
    // Coerce all values > 0 to `true`.
    return val === 'true' || Number(val) >= 1 || val === true || false;
  }
  fromColumn(val: unknown) {
    return Number(val) >= 1 || false;
  }
  columnSQL() {
    const defaultValue = this.defaultValue ? 1 : 0;
    return `${this.tableColumn} INTEGER DEFAULT ${defaultValue}`;
  }
  equal(val: string | number | boolean) {
    // equal(true) matches all values != 0
    this._assertPresentAndQueryable('equal', val);
    return new Matcher(this, val ? '!=' : '=', 0);
  }
}
