import { describe, expect, it } from 'vitest';
import {
  dedupeContacts,
  endsWithSeparator,
  formatContact,
  parseAddress,
  parseRecipients,
  splitAddressList,
} from './recipients';

describe('parseAddress', () => {
  it('reads bare, angled and named addresses', () => {
    expect(parseAddress('ann@example.com')).toEqual({ email: 'ann@example.com' });
    expect(parseAddress('<ann@example.com>')).toEqual({ email: 'ann@example.com' });
    expect(parseAddress('Ann Lee <ann@example.com>')).toEqual({
      name: 'Ann Lee',
      email: 'ann@example.com',
    });
    expect(parseAddress('"Lee, Ann" <ann@example.com>')).toEqual({
      name: 'Lee, Ann',
      email: 'ann@example.com',
    });
  });

  it('strips mailto links', () => {
    expect(parseAddress('mailto:ann@example.com?subject=Hi')).toEqual({ email: 'ann@example.com' });
  });

  it('rejects things that are not addresses', () => {
    expect(parseAddress('ann')).toBeNull();
    expect(parseAddress('ann@example')).toBeNull();
    expect(parseAddress('Ann <not an address>')).toBeNull();
    expect(parseAddress('a b@example.com')).toBeNull();
  });
});

describe('parseRecipients', () => {
  it('splits on commas, semicolons and newlines outside quotes', () => {
    const { contacts, invalid } = parseRecipients(
      '"Doe, Jane" <jane@x.io>; bob@y.org\nCarol <carol@z.net>, nope',
    );
    expect(contacts).toEqual([
      { name: 'Doe, Jane', email: 'jane@x.io' },
      { email: 'bob@y.org' },
      { name: 'Carol', email: 'carol@z.net' },
    ]);
    expect(invalid).toEqual(['nope']);
  });

  it('dedupes by address, case-insensitively, keeping a name if one shows up', () => {
    const { contacts } = parseRecipients('BOB@y.org, Bob <bob@y.org>');
    expect(contacts).toEqual([{ email: 'BOB@y.org', name: 'Bob' }]);
  });

  it('ignores empty tokens', () => {
    expect(parseRecipients(' , ;; ').contacts).toEqual([]);
    expect(splitAddressList('a@b.co,,')).toEqual(['a@b.co']);
  });
});

describe('helpers', () => {
  it('drops excluded addresses', () => {
    expect(dedupeContacts([{ email: 'me@x.io' }, { email: 'you@x.io' }], ['ME@x.io'])).toEqual([
      { email: 'you@x.io' },
    ]);
  });

  it('formats contacts and quotes names with separators', () => {
    expect(formatContact({ email: 'a@b.co' })).toBe('a@b.co');
    expect(formatContact({ name: 'Ann', email: 'a@b.co' })).toBe('Ann <a@b.co>');
    expect(formatContact({ name: 'Lee, Ann', email: 'a@b.co' })).toBe('"Lee, Ann" <a@b.co>');
    expect(parseAddress(formatContact({ name: 'Lee, "A"', email: 'a@b.co' }))).toEqual({
      name: 'Lee, "A"',
      email: 'a@b.co',
    });
  });

  it('detects a trailing separator only outside quotes', () => {
    expect(endsWithSeparator('ann@x.io,')).toBe(true);
    expect(endsWithSeparator('ann@x.io; ')).toBe(true);
    expect(endsWithSeparator('"Lee,')).toBe(false);
    expect(endsWithSeparator('ann@x.io')).toBe(false);
  });
});
