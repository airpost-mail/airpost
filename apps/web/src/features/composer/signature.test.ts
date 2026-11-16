import { beforeEach, describe, expect, it } from 'vitest';
import {
  createSignature,
  resetSignatures,
  setDefaultSignature,
  updateSignature,
} from '@/features/settings/signatures/store';
import { draftFromMessage } from './draft';
import { isBlankHtml } from './html';
import { withoutSignature, withSignature } from './signature';

const newDraft = () =>
  draftFromMessage({
    id: 'd1',
    aid: 'a1',
    v: 0,
    __cls: 'Message',
    subject: '',
    date: 0,
    threadId: '',
    hMsgId: 'd1@airpost',
    unread: false,
    starred: false,
    draft: true,
    body: '',
    from: [{ name: 'Maya', email: 'maya@okafor.studio' }],
    to: [],
    cc: [],
    bcc: [],
    replyTo: [],
    files: [],
  });

beforeEach(() => resetSignatures());

describe('signatures in drafts', () => {
  it('appends the sender’s default signature, sanitised', () => {
    const sig = createSignature('Work');
    updateSignature(sig.id, { body: '<p onclick="x()">Maya Okafor</p><script>bad()</script>' });
    setDefaultSignature('maya@okafor.studio', sig.id);

    const body = withSignature(newDraft()).body;
    expect(body).toContain('Maya Okafor');
    expect(body).not.toMatch(/onclick|script/);
    expect(isBlankHtml(withoutSignature(body))).toBe(true);
  });

  it('leaves the draft alone without a default', () => {
    const draft = newDraft();
    expect(withSignature(draft)).toBe(draft);
  });
});
