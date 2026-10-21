// The part of the ini package (no bundled types) linux-theme.ts uses. Values are strings,
// booleans or nested sections, as the file has them.
declare module 'ini' {
  export function parse(str: string): { [key: string]: unknown };
  const ini: { parse: typeof parse };
  export default ini;
}
