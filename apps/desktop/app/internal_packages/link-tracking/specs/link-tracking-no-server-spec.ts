import { Message, IdentityStore } from 'mailspring-exports';

import LinkTrackingComposerExtension from '../lib/link-tracking-composer-extension';
import { PLUGIN_ID, PLUGIN_URL } from '../lib/link-tracking-constants';

const body = `TEST_BODY<br><a href="http://example.com">link</a>`;

const nodeForHTML = (html: string) => {
  const node = document.createElement('root');
  document.createDocumentFragment().appendChild(node);
  node.innerHTML = html;
  return node;
};

describe('Link tracking composer extension without a tracking server', () => {
  let draft: Message;
  let root: HTMLElement;

  beforeEach(() => {
    draft = new Message({ accountId: 'test', headerMessageId: 'hmid-1', body });
    draft.directlyAttachMetadata(PLUGIN_ID, { tracked: true });
    root = nodeForHTML(body);
  });

  it('leaves links alone and needs no per-recipient bodies when signed out', () => {
    spyOn(IdentityStore, 'identity').andReturn(null);
    LinkTrackingComposerExtension.applyTransformsForSending({ draftBodyRootNode: root, draft });
    expect(root.innerHTML).toEqual(body);
    expect(LinkTrackingComposerExtension.needsPerRecipientBodies(draft)).toBe(false);
  });

  it('rewrites links when an identity is present', () => {
    spyOn(IdentityStore, 'identity').andReturn({ id: 'x' } as any);
    LinkTrackingComposerExtension.applyTransformsForSending({ draftBodyRootNode: root, draft });
    expect(root.innerHTML).toContain(`${PLUGIN_URL}/link/hmid-1/0?redirect=`);
    expect(LinkTrackingComposerExtension.needsPerRecipientBodies(draft)).toBe(true);
  });
});
