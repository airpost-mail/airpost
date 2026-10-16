import { beforeEach, describe, expect, it } from 'vitest';
import {
  createSignature,
  resetSignatures,
  setDefaultSignature,
  updateSignature,
} from '@/features/settings/signatures/store';
import { newDraft } from './draft';
import { isBlankHtml } from './html';
import { withoutSignature, withSignature } from './signature';

const identity = { accountId: 'a1', contact: { name: 'Maya', email: 'maya@okafor.studio' } };

beforeEach(() => resetSignatures());

describe('signatures in drafts', () => {
  it('appends the sender’s default signature, sanitised', () => {
    const sig = createSignature('Work');
    updateSignature(sig.id, { body: '<p onclick="x()">Maya Okafor</p><script>bad()</script>' });
    setDefaultSignature('maya@okafor.studio', sig.id);

    const body = withSignature(newDraft(identity)).body;
    expect(body).toContain('Maya Okafor');
    expect(body).not.toMatch(/onclick|script/);
    expect(isBlankHtml(withoutSignature(body))).toBe(true);
  });

  it('leaves the draft alone without a default', () => {
    const draft = newDraft(identity);
    expect(withSignature(draft)).toBe(draft);
  });
});
