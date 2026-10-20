import _ from 'underscore';
import { Contact } from '../../src/data/models/contact';
import ContactStore from '../../src/data/stores/contact';

xdescribe('ContactStore', function () {
  beforeEach(function () {
    spyOn(AppEnv, 'isMainWindow').andReturn(true);
  });

  describe('when searching for a contact', function () {
    beforeEach(function () {
      this.c1 = new Contact({ name: '', email: '1test@example.com', refs: 7 });
      this.c2 = new Contact({ name: 'First', email: '2test@example.com', refs: 6 });
      this.c3 = new Contact({ name: 'First Last', email: '3test@example.com', refs: 5 });
      this.c4 = new Contact({ name: 'Fit', email: 'fit@example.com', refs: 4 });
      this.c5 = new Contact({ name: 'Fins', email: 'fins@example.com', refs: 3 });
      this.c6 = new Contact({ name: 'Fill', email: 'fill@example.com', refs: 2 });
      this.c7 = new Contact({ name: 'Fin', email: 'fin@example.com', refs: 1 });
    });

    it('can find by first name', function () {
      waitsForPromise(() => {
        return ContactStore.searchContacts('First').then((results) => {
          expect(results.length).toBe(2);
          expect(results[0]).toBe(this.c2);
          expect(results[1]).toBe(this.c3);
        });
      });
    });

    it('can find by last name', function () {
      waitsForPromise(() => {
        return ContactStore.searchContacts('Last').then((results) => {
          expect(results.length).toBe(1);
          expect(results[0]).toBe(this.c3);
        });
      });
    });

    it('can find by email', function () {
      waitsForPromise(() => {
        return ContactStore.searchContacts('1test').then((results) => {
          expect(results.length).toBe(1);
          expect(results[0]).toBe(this.c1);
        });
      });
    });

    it('is case insensitive', function () {
      waitsForPromise(() => {
        return ContactStore.searchContacts('FIrsT').then((results) => {
          expect(results.length).toBe(2);
          expect(results[0]).toBe(this.c2);
          expect(results[1]).toBe(this.c3);
        });
      });
    });

    it('only returns the number requested', function () {
      waitsForPromise(() => {
        return ContactStore.searchContacts('FIrsT', { limit: 1 }).then((results) => {
          expect(results.length).toBe(1);
          expect(results[0]).toBe(this.c2);
        });
      });
    });

    it('returns no more than 5 by default', () =>
      waitsForPromise(() => {
        return ContactStore.searchContacts('fi').then((results) => {
          expect(results.length).toBe(5);
        });
      }));

    it('can return more than 5 if requested', () =>
      waitsForPromise(() => {
        return ContactStore.searchContacts('fi', { limit: 6 }).then((results) => {
          expect(results.length).toBe(6);
        });
      }));
  });

  describe('isValidContact', function () {
    it('should call contact.isValid', function () {
      const contact = new Contact({} as any);
      spyOn(contact, 'isValid').andReturn(true);
      expect(ContactStore.isValidContact(contact)).toBe(true);
    });

    it('should return false for non-Contact objects', () =>
      expect(
        ContactStore.isValidContact({
          name: 'Ben',
          email: 'ben@example.com',
        } as unknown as Contact)
      ).toBe(false));

    it("returns false if we're not passed a contact", () =>
      expect(ContactStore.isValidContact(undefined as unknown as Contact)).toBe(false));
  });

  describe('parseContactsInString', function () {
    const testCases = {
      // Single contact test cases
      'evan@example.com': [new Contact({ name: 'evan@example.com', email: 'evan@example.com' })],
      'Evan Morikawa': [],
      "'evan@example.com'": [new Contact({ name: 'evan@example.com', email: 'evan@example.com' })],
      '"evan@example.com"': [new Contact({ name: 'evan@example.com', email: 'evan@example.com' })],
      "'evan@example.com": [new Contact({ name: "'evan@example.com", email: "'evan@example.com" })],
      'Evan Morikawa <evan@example.com>': [
        new Contact({ name: 'Evan Morikawa', email: 'evan@example.com' }),
      ],
      'Evan Morikawa (evan@example.com)': [
        new Contact({ name: 'Evan Morikawa', email: 'evan@example.com' }),
      ],
      'spang (Christine Spang) <noreply+phabricator@nilas.com>': [
        new Contact({ name: 'spang (Christine Spang)', email: 'noreply+phabricator@nilas.com' }),
      ],
      "spang 'Christine Spang' <noreply+phabricator@nilas.com>": [
        new Contact({ name: "spang 'Christine Spang'", email: 'noreply+phabricator@nilas.com' }),
      ],
      'spang "Christine Spang" <noreply+phabricator@nilas.com>': [
        new Contact({ name: 'spang "Christine Spang"', email: 'noreply+phabricator@nilas.com' }),
      ],
      'Evan (evan@example.com)': [new Contact({ name: 'Evan', email: 'evan@example.com' })],
      '"Michael" (mg@example.com)': [new Contact({ name: 'Michael', email: 'mg@example.com' })],
      'announce-uc.1440659566.kankcagcmaacemjlnoma-security=example.com@lists.openwall.com': [
        new Contact({
          name: 'announce-uc.1440659566.kankcagcmaacemjlnoma-security=example.com@lists.openwall.com',
          email:
            'announce-uc.1440659566.kankcagcmaacemjlnoma-security=example.com@lists.openwall.com',
        }),
      ],

      // Multiple contact test cases
      'Evan Morikawa <evan@example.com>, Ben <ben@example.com>': [
        new Contact({ name: 'Evan Morikawa', email: 'evan@example.com' }),
        new Contact({ name: 'Ben', email: 'ben@example.com' }),
      ],
      'Evan Morikawa <evan@example.com>; Ben <ben@example.com>': [
        new Contact({ name: 'Evan Morikawa', email: 'evan@example.com' }),
        new Contact({ name: 'Ben', email: 'ben@example.com' }),
      ],
      'mark@example.com\nGleb (gleb@example.com)\rEvan Morikawa <evan@example.com>, spang (Christine Spang) <noreply+phabricator@nilas.com>':
        [
          new Contact({ name: '', email: 'mark@example.com' }),
          new Contact({ name: 'Gleb', email: 'gleb@example.com' }),
          new Contact({ name: 'Evan Morikawa', email: 'evan@example.com' }),
          new Contact({ name: 'spang (Christine Spang)', email: 'noreply+phabricator@nilas.com' }),
        ],
    };

    _.forEach(testCases, (value, key) =>
      it(`works for ${key}`, () =>
        waitsForPromise(() =>
          ContactStore.parseContactsInString(key).then(function (contacts) {
            const contactStrings = contacts.map((c) => c.toString());
            const expectedContacts = value.map((c) => c.toString());
            expect(contactStrings).toEqual(expectedContacts);
          })
        ))
    );
  });
});
