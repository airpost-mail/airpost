import { describe, expect, it } from 'vitest';
import type { CategoryJSON, ContactJSON } from '@/engine';
import { suggest, type SuggestContext } from './suggest';

const cat = (id: string, path: string, role: CategoryJSON['role'] = ''): CategoryJSON => ({
  id,
  aid: 'a1',
  v: 1,
  __cls: 'Folder',
  path,
  role,
});

const contact = (name: string, email: string): ContactJSON => ({
  id: email,
  aid: 'a1',
  v: 1,
  __cls: 'Contact',
  name,
  email,
  refs: 1,
  gis: [],
  info: {},
});

const ctx: SuggestContext = {
  contacts: [contact('Ana Lima', 'ana@x.example')],
  categories: [
    { category: cat('f1', 'INBOX', 'inbox'), accountName: 'Work' },
    { category: cat('f2', 'Clients/Harbour Labs'), accountName: 'Work' },
  ],
  recent: ['from:ana invoice'],
  multipleAccounts: false,
  now: new Date(2026, 9, 16),
};

describe('suggest', () => {
  it('offers recent searches and every operator for an empty box', () => {
    const s = suggest('', ctx);
    expect(s[0]).toMatchObject({ kind: 'recent', value: 'from:ana invoice', submit: true });
    expect(s.filter((x) => x.kind === 'operator').map((x) => x.label)).toContain('has:');
  });

  it('completes operator names from a prefix', () => {
    expect(suggest('report fr', ctx).map((s) => s.value)).toEqual(['report from:']);
  });

  it('completes is: and has: values', () => {
    expect(suggest('is:st', ctx).map((s) => s.value)).toEqual(['is:starred ']);
    expect(suggest('has:', ctx).map((s) => s.value)).toEqual(['has:attachment ']);
  });

  it('completes in: with roles and quoted folder names', () => {
    expect(suggest('in:', ctx).map((s) => s.value)).toEqual([
      'in:inbox ',
      'in:"Clients/Harbour Labs" ',
    ]);
    expect(suggest('x in:harb', ctx).map((s) => s.label)).toEqual(['Clients/Harbour Labs']);
  });

  it('completes from: with contacts', () => {
    expect(suggest('from:an', ctx)[0]).toMatchObject({
      label: 'Ana Lima',
      hint: 'ana@x.example',
      value: 'from:ana@x.example ',
    });
  });

  it('offers date presets with the date they mean', () => {
    const s = suggest('after:', ctx);
    expect(s.map((x) => x.label)).toEqual(['today', 'yesterday', '7d', '30d', '1y']);
    expect(s[2].hint).toBe('Fri 9 Oct 2026');
  });
});
