import { beforeEach, describe, expect, it } from 'vitest';
import { getComposerWindows, openComposer, resetComposers, setComposerEditingId } from './store';

describe('composer store', () => {
  beforeEach(() => resetComposers());

  it('focuses the reply window already editing a draft instead of opening a second one', () => {
    const reply = openComposer({ mode: 'reply', threadId: 't1' });
    setComposerEditingId(reply, 'draft-x');
    const again = openComposer({ mode: 'edit-draft', draftId: 'draft-x' });
    expect(again).toBe(reply);
    expect(getComposerWindows()).toHaveLength(1);
  });
});
