import { Attribute } from './attribute';
import { Matcher } from './matcher';

/*
Public: The value of this attribute is always a string or `null`.

String attributes can be queries using `equal`, `not`, and `startsWith`. Matching on
`greaterThan` and `lessThan` is not supported.

Section: Database
*/
export class AttributeString extends Attribute {
  // `null` is omitted rather than serialized, so that null and absent mean the same
  // thing to the engine.
  toJSON(val: string | null): string | undefined {
    return val === null ? undefined : val;
  }

  fromJSON(val: unknown) {
    return val === null || val === undefined || val === false ? null : `${val}`;
  }

  // Public: Returns a {Matcher} for objects starting with the provided value.
  startsWith(val: string) {
    return new Matcher(this, 'startsWith', val);
  }

  columnSQL() {
    return `${this.tableColumn} TEXT`;
  }

  like(val: string) {
    this._assertPresentAndQueryable('like', val);
    return new Matcher(this, 'like', val);
  }

  lessThan(val: string) {
    this._assertPresentAndQueryable('lessThanOrEqualTo', val);
    return new Matcher(this, '<', val);
  }

  lessThanOrEqualTo(val: string) {
    this._assertPresentAndQueryable('lessThanOrEqualTo', val);
    return new Matcher(this, '<=', val);
  }

  greaterThan(val: string) {
    this._assertPresentAndQueryable('greaterThanOrEqualTo', val);
    return new Matcher(this, '>', val);
  }

  greaterThanOrEqualTo(val: string) {
    this._assertPresentAndQueryable('greaterThanOrEqualTo', val);
    return new Matcher(this, '>=', val);
  }
}
