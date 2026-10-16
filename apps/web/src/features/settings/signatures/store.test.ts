import { beforeEach, describe, expect, it } from 'vitest';
import {
  createSignature,
  getSignatures,
  removeSignature,
  resetSignatures,
  setDefaultSignature,
  signatureForEmail,
  updateSignature,
} from './store';

beforeEach(() => resetSignatures());

describe('signatures store', () => {
  it('creates, edits and persists signatures', () => {
    const s = createSignature('Work');
    updateSignature(s.id, { body: '<p>Maya</p>' });
    expect(getSignatures().signatures[s.id]).toEqual({
      id: s.id,
      title: 'Work',
      body: '<p>Maya</p>',
    });
    resetSignatures();
    expect(getSignatures().signatures[s.id].body).toBe('<p>Maya</p>');
  });

  it('resolves the default per email and clears it when the signature goes', () => {
    const s = createSignature('Work');
    setDefaultSignature('maya@okafor.studio', s.id);
    expect(signatureForEmail('maya@okafor.studio')?.id).toBe(s.id);
    expect(signatureForEmail('other@example.org')).toBeNull();
    removeSignature(s.id);
    expect(getSignatures().defaults).toEqual({});
    expect(signatureForEmail('maya@okafor.studio')).toBeNull();
  });
});
