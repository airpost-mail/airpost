import { Attribute } from './attribute';
import { Matcher } from './matcher';

/*
Public: The value of this attribute is always a number, or null.

Section: Database
*/
export class AttributeNumber extends Attribute {
  toJSON(val: unknown) {
    return val;
  }

  fromJSON(val: unknown) {
    // isNaN coerces the same way Number() does.
    return isNaN(Number(val)) ? null : Number(val);
  }

  columnSQL() {
    return `${this.tableColumn} INTEGER`;
  }

  // Public: Returns a {Matcher} for objects greater than the provided value.
  greaterThan(val: number) {
    this._assertPresentAndQueryable('greaterThan', val);
    return new Matcher(this, '>', val);
  }

  // Public: Returns a {Matcher} for objects less than the provided value.
  lessThan(val: number) {
    this._assertPresentAndQueryable('lessThan', val);
    return new Matcher(this, '<', val);
  }

  // Public: Returns a {Matcher} for objects greater than the provided value.
  greaterThanOrEqualTo(val: number) {
    this._assertPresentAndQueryable('greaterThanOrEqualTo', val);
    return new Matcher(this, '>=', val);
  }

  // Public: Returns a {Matcher} for objects less than the provided value.
  lessThanOrEqualTo(val: number) {
    this._assertPresentAndQueryable('lessThanOrEqualTo', val);
    return new Matcher(this, '<=', val);
  }

  gt = AttributeNumber.prototype.greaterThan;
  lt = AttributeNumber.prototype.lessThan;
  gte = AttributeNumber.prototype.greaterThanOrEqualTo;
  lte = AttributeNumber.prototype.lessThanOrEqualTo;
}
