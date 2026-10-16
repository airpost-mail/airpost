import { beforeEach, describe, expect, it } from 'vitest';
import {
  closeComposer,
  dockComposer,
  focusComposer,
  getComposerWindows,
  isInlineDraftDismissed,
  openComposer,
  registerInlineSlot,
  resetComposers,
  setComposerEditingId,
  setComposerHidden,
} from './store';

const only = () => {
  const list = getComposerWindows();
  expect(list).toHaveLength(1);
  return list[0];
};

describe('composer store', () => {
  beforeEach(() => resetComposers());

  it('focuses the reply window already editing a draft instead of opening a second one', () => {
    const reply = openComposer({ mode: 'reply', threadId: 't1' });
    setComposerEditingId(reply, 'draft-x');
    const again = openComposer({ mode: 'edit-draft', draftId: 'draft-x' });
    expect(again).toBe(reply);
    expect(getComposerWindows()).toHaveLength(1);
  });

  it('docks a reply whose thread is not open in the reader', () => {
    openComposer({ mode: 'reply', threadId: 't1' });
    expect(only().placement).toBe('docked');
  });

  it('puts replies and forwards inline when the thread is on screen', () => {
    registerInlineSlot('t1');
    openComposer({ mode: 'forward', threadId: 't1' });
    expect(only().placement).toBe('inline');
  });

  it('keeps one inline reply per thread and focuses it when asked again', () => {
    registerInlineSlot('t1');
    const first = openComposer({ mode: 'reply', threadId: 't1', messageId: 'm1' });
    const seq = only().focusSeq;
    expect(openComposer({ mode: 'reply-all', threadId: 't1', messageId: 'm2' })).toBe(first);
    expect(only().focusSeq).toBe(seq + 1);
  });

  it('docks new messages and standalone drafts even with the thread open', () => {
    registerInlineSlot('t1');
    openComposer({ mode: 'new' });
    openComposer({ mode: 'edit-draft', threadId: 't1', draftId: 'd1', placement: 'docked' });
    expect(getComposerWindows().map((w) => w.placement)).toEqual(['docked', 'docked']);
  });

  it('opens a resumed draft inline without focus', () => {
    registerInlineSlot('t1');
    openComposer({
      mode: 'edit-draft',
      threadId: 't1',
      draftId: 'd1',
      placement: 'inline',
      focus: false,
    });
    expect(only()).toMatchObject({ placement: 'inline', focusSeq: 0 });
  });

  it('closes inline replies when their thread is left', () => {
    const release = registerInlineSlot('t1');
    const key = openComposer({ mode: 'reply', threadId: 't1' });
    setComposerEditingId(key, 'd1');
    release();
    expect(getComposerWindows()).toHaveLength(0);
    // Not dismissed: coming back to the thread shows the saved draft again.
    expect(isInlineDraftDismissed('d1')).toBe(false);
  });

  it('turns a reply with a pending send into an edit of its draft, docked on undo', () => {
    const release = registerInlineSlot('t1');
    const key = openComposer({ mode: 'reply', threadId: 't1' });
    setComposerEditingId(key, 'd1');
    setComposerHidden(key, true);
    release();
    expect(only()).toMatchObject({ mode: 'edit-draft', draftId: 'd1', placement: 'inline' });
    focusComposer(key);
    expect(only()).toMatchObject({ placement: 'docked', hidden: false });
  });

  it('pops an inline reply out as its saved draft', () => {
    registerInlineSlot('t1');
    const key = openComposer({ mode: 'reply', threadId: 't1' });
    dockComposer(key, 'd1');
    expect(only()).toMatchObject({ placement: 'docked', mode: 'edit-draft', draftId: 'd1' });
  });

  it('remembers closed drafts until they are opened again', () => {
    const key = openComposer({ mode: 'reply', threadId: 't1' });
    setComposerEditingId(key, 'd1');
    closeComposer(key);
    expect(isInlineDraftDismissed('d1')).toBe(true);
    openComposer({ mode: 'edit-draft', draftId: 'd1' });
    expect(isInlineDraftDismissed('d1')).toBe(false);
  });
});
