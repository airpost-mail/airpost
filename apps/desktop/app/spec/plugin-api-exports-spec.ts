// Every key of the two plugin API barrels resolves through a lazy getter that requires a path
// string, so a moved or renamed module only fails when some plugin first reads that key.
const BARRELS = ['mailspring-exports', 'mailspring-component-kit'];

describe('plugin API barrels', () => {
  for (const barrel of BARRELS) {
    it(`loads every export of ${barrel}`, () => {
      const exportsObject = require(barrel);
      const keys = Object.keys(exportsObject);
      expect(keys.length).toBeGreaterThan(0);

      const failures: string[] = [];
      for (const key of keys) {
        try {
          if (exportsObject[key] === undefined) failures.push(`${key}: undefined`);
        } catch (error) {
          failures.push(`${key}: ${error.message}`);
        }
      }
      expect(failures).toEqual([]);
    });
  }
});
