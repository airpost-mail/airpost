import { describe, expect, it } from 'vitest';
import { parseServerUrl, safeRedirect, serverLabel } from '../server-url';

describe('parseServerUrl', () => {
  it('adds https and drops trailing slashes', () => {
    expect(parseServerUrl('tern.example.com/')).toEqual({
      ok: true,
      url: 'https://tern.example.com',
    });
    expect(parseServerUrl('https://x.example/tern/')).toEqual({
      ok: true,
      url: 'https://x.example/tern',
    });
  });

  it('allows http only on loopback', () => {
    expect(parseServerUrl('http://localhost:5173')).toEqual({
      ok: true,
      url: 'http://localhost:5173',
    });
    expect(parseServerUrl('http://127.0.0.1:8080').ok).toBe(true);
    expect(parseServerUrl('http://tern.example.com').ok).toBe(false);
  });

  it('rejects other schemes, blanks and embedded credentials', () => {
    expect(parseServerUrl('ftp://x.example').ok).toBe(false);
    expect(parseServerUrl('  ').ok).toBe(false);
    expect(parseServerUrl('https://u:p@x.example').ok).toBe(false);
  });

  it('labels by host', () => {
    expect(serverLabel('https://tern.example.com')).toBe('tern.example.com');
    expect(serverLabel('http://localhost:5173')).toBe('localhost:5173');
  });
});

describe('safeRedirect', () => {
  it('keeps same-origin paths', () => {
    expect(safeRedirect('/acc/inbox?x=1')).toBe('/acc/inbox?x=1');
  });

  it('drops anything that could leave the origin or loop', () => {
    for (const v of ['//evil.example', '/\\evil.example', 'https://evil.example', '/login', 42]) {
      expect(safeRedirect(v)).toBeUndefined();
    }
  });
});
