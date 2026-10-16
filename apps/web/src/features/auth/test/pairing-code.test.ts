import { describe, expect, it } from 'vitest';
import {
  codeFromHash,
  defaultDeviceName,
  formatPairingCode,
  normalizePairingCode,
} from '../pairing-code';
import { safeRedirect } from '../redirect';

describe('normalizePairingCode', () => {
  it('accepts groups, spaces and lower case', () => {
    expect(normalizePairingCode('ABCD-EFGH-JKLM')).toBe('ABCDEFGHJKLM');
    expect(normalizePairingCode(' abcd efgh jklm ')).toBe('ABCDEFGHJKLM');
    expect(normalizePairingCode('abcdefgh2345')).toBe('ABCDEFGH2345');
  });

  it('refuses the wrong length and characters outside base32', () => {
    expect(normalizePairingCode('ABCD-EFGH')).toBeNull();
    expect(normalizePairingCode('ABCD-EFGH-JKLMN')).toBeNull();
    expect(normalizePairingCode('ABCD-EFGH-JK01')).toBeNull();
    expect(normalizePairingCode('ABCD_EFGH_JKLM')).toBeNull();
    expect(normalizePairingCode('')).toBeNull();
  });

  it('formats in groups of four', () => {
    expect(formatPairingCode('ABCDEFGHJKLM')).toBe('ABCD-EFGH-JKLM');
  });

  it('reads a code from a /pair#code fragment', () => {
    expect(codeFromHash('#ABCD-EFGH-JKLM')).toBe('ABCDEFGHJKLM');
    expect(codeFromHash('#abcd%20efgh%20jklm')).toBe('ABCDEFGHJKLM');
    expect(codeFromHash('#nope')).toBeNull();
    expect(codeFromHash('')).toBeNull();
  });
});

describe('defaultDeviceName', () => {
  it('names the browser and the OS', () => {
    expect(
      defaultDeviceName(
        'Mozilla/5.0 (Macintosh; Intel Mac OS X 14.5; rv:131.0) Gecko/20100101 Firefox/131.0',
      ),
    ).toBe('Firefox on macOS');
    expect(
      defaultDeviceName(
        'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36 Edg/130.0',
      ),
    ).toBe('Edge on Windows');
    expect(defaultDeviceName('curl/8')).toBe('Browser');
  });
});

describe('safeRedirect', () => {
  it('keeps paths on this origin only', () => {
    expect(safeRedirect('/inbox?x=1')).toBe('/inbox?x=1');
    expect(safeRedirect('//evil.example')).toBeUndefined();
    expect(safeRedirect('/\\evil.example')).toBeUndefined();
    expect(safeRedirect('https://evil.example')).toBeUndefined();
    expect(safeRedirect('/pair')).toBeUndefined();
    expect(safeRedirect(3)).toBeUndefined();
  });
});
