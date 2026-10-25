import _ from 'underscore';

import { File } from '../../src/data/models/file';
import * as Actions from '../../src/data/actions';
import { Thread } from '../../src/data/models/thread';
import { Contact } from '../../src/data/models/contact';
import { Message } from '../../src/data/models/message';
import DraftStore from '../../src/data/drafts/store';
import { AccountStore } from '../../src/data/stores/account';
import DatabaseStore from '../../src/data/database/store';
import AttachmentStore from '../../src/data/stores/attachment';
import { MailboxPerspective } from '../../src/data/mailbox-perspective';
import SanitizeTransformer from '../../src/mail/body/sanitize-transformer';
import FocusedContentStore from '../../src/data/stores/focused-content';
import InlineStyleTransformer from '../../src/mail/body/inline-style-transformer';
import FocusedPerspectiveStore from '../../src/data/stores/focused-perspective';

import os from 'os';
import DraftFactory, { generateHeaderMessageId } from '../../src/data/drafts/factory';

let msgFromMe = null;
let fakeThread = null;
let fakeMessage1 = null;
let msgWithReplyTo = null;
let fakeMessageWithFiles = null;
let msgWithReplyToDuplicates = null;
let msgWithReplyToFromMe = null;
let account = null;
const downloadData = {};

const expectContactsEqual = (a, b) => {
  expect(a.map((c) => c.email).sort()).toEqual(b.map((c) => c.email).sort());
};

describe('DraftFactory', function draftFactory() {
  beforeEach(() => {
    // Out of the scope of these specs
    spyOn(InlineStyleTransformer, 'run').andCallFake((input) => Promise.resolve(input));
    spyOn(SanitizeTransformer, 'run').andCallFake((input) => Promise.resolve(input));
    spyOn(AttachmentStore, 'getDownloadDataForFile').andCallFake((fid) => {
      return downloadData[fid];
    });

    account = AccountStore.accounts()[0];
    const files = [
      new File({ filename: 'test.jpg', accountId: account.id, id: 'a' }),
      new File({ filename: 'test.pdj', accountId: account.id, id: 'b' }),
    ];
    files.forEach((file) => {
      downloadData[file.id] = {
        fileId: file.id,
        filename: file.filename,
      };
    });

    fakeThread = new Thread({
      id: 'fake-thread-id',
      accountId: account.id,
      subject: 'Fake Subject',
    });

    fakeMessage1 = new Message({
      id: 'fake-message-1',
      headerMessageId: 'fake-message-1@localhost',
      accountId: account.id,
      to: [new Contact({ email: 'ben@example.com' }), new Contact({ email: 'evan@example.com' })],
      cc: [new Contact({ email: 'mg@example.com' }), account.me()],
      bcc: [new Contact({ email: 'recruiting@example.com' })],
      from: [new Contact({ email: 'customer@example.com', name: 'Customer' })],
      threadId: 'fake-thread-id',
      body: 'Fake Message 1',
      subject: 'Fake Subject',
      date: new Date(1415814587),
    });

    fakeMessageWithFiles = new Message({
      id: 'fake-message-with-files',
      headerMessageId: 'fake-message-with-files@localhost',
      accountId: account.id,
      to: [new Contact({ email: 'ben@example.com' }), new Contact({ email: 'evan@example.com' })],
      cc: [new Contact({ email: 'mg@example.com' }), account.me()],
      bcc: [new Contact({ email: 'recruiting@example.com' })],
      from: [new Contact({ email: 'customer@example.com', name: 'Customer' })],
      files: files,
      threadId: 'fake-thread-id',
      body: 'Fake Message 1',
      subject: 'Fake Subject',
      date: new Date(1415814587),
    });

    msgFromMe = new Message({
      id: 'fake-message-3',
      headerMessageId: 'fake-message-3@localhost',
      accountId: account.id,
      to: [new Contact({ email: '1@1.com' }), new Contact({ email: '2@2.com' })],
      cc: [new Contact({ email: '3@3.com' }), new Contact({ email: '4@4.com' })],
      bcc: [new Contact({ email: '5@5.com' }), new Contact({ email: '6@6.com' })],
      from: [account.me()],
      threadId: 'fake-thread-id',
      body: 'Fake Message 2',
      subject: 'Re: Fake Subject',
      date: new Date(1415814587),
    });

    msgWithReplyTo = new Message({
      id: 'fake-message-reply-to',
      headerMessageId: 'fake-message-reply-to@localhost',
      accountId: account.id,
      to: [new Contact({ email: '1@1.com' }), new Contact({ email: '2@2.com' })],
      cc: [new Contact({ email: '3@3.com' }), new Contact({ email: '4@4.com' })],
      bcc: [new Contact({ email: '5@5.com' }), new Contact({ email: '6@6.com' })],
      replyTo: [new Contact({ email: 'reply-to@5.com' }), new Contact({ email: 'reply-to@6.com' })],
      from: [new Contact({ email: 'from@5.com' })],
      threadId: 'fake-thread-id',
      body: 'Fake Message 2',
      subject: 'Re: Fake Subject',
      date: new Date(1415814587),
    });

    msgWithReplyToFromMe = new Message({
      accountId: account.id,
      threadId: 'fake-thread-id',
      from: [account.me()],
      to: [new Contact({ email: 'tiffany@popular.com', name: 'Tiffany' })],
      replyTo: [new Contact({ email: 'danco@gmail.com', name: 'danco@gmail.com' })],
    });

    msgWithReplyToDuplicates = new Message({
      id: 'fake-message-reply-to-duplicates',
      headerMessageId: 'fake-message-reply-to-duplicates@localhost',
      accountId: account.id,
      to: [new Contact({ email: '1@1.com' }), new Contact({ email: '2@2.com' })],
      cc: [new Contact({ email: '1@1.com' }), new Contact({ email: '4@4.com' })],
      from: [new Contact({ email: 'reply-to@5.com' })],
      replyTo: [new Contact({ email: 'reply-to@5.com' })],
      threadId: 'fake-thread-id',
      body: 'Fake Message Duplicates',
      subject: 'Re: Fake Subject',
      date: new Date(1415814587),
    });
  });

  describe('creating drafts', () => {
    describe('createDraftForSendAgain', () => {
      it('creates a new editable message with the original contents and recipients', () => {
        spyOn(Actions, 'fetchFile');

        waitsForPromise(() =>
          DraftFactory.createDraftForSendAgain(fakeMessageWithFiles).then((draft) => {
            expect(draft.threadId).toBeFalsy();
            expect(draft.replyToHeaderMessageId).toBeFalsy();
            expect(draft.forwardedHeaderMessageId).toBeFalsy();
            expect(draft.subject).toBe(fakeMessageWithFiles.subject);
            expect(draft.body).toBe(fakeMessageWithFiles.body);
            expect(draft.to).toEqual(fakeMessageWithFiles.to);
            expect(draft.cc).toEqual(fakeMessageWithFiles.cc);
            expect(draft.bcc).toEqual(fakeMessageWithFiles.bcc);
            expect(draft.from).toEqual(fakeMessageWithFiles.from);
            expect(draft.files).toEqual(fakeMessageWithFiles.files);
            expect(Actions.fetchFile).toHaveBeenCalled();
          })
        );
      });
    });
  });

  describe('createOrUpdateDraftForReply', () => {
    it('should throw an exception unless you provide `reply` or `reply-all`', () => {
      waitsForPromise(() =>
        DraftFactory.createOrUpdateDraftForReply({
          thread: fakeThread,
          message: fakeMessage1,
          type: 'wrong' as any,
          behavior: 'prefer-existing',
        })
          .then(() => {
            expect(false).toEqual(true);
          })
          .catch(() => {
            expect(true).toEqual(true);
          })
      );
    });

    describe('when there is already a draft in reply to the same message the thread', () => {
      beforeEach(() => {
        this.existingDraft = new Message({
          id: 'asd',
          accountId: TEST_ACCOUNT_ID,
          replyToHeaderMessageId: fakeMessage1.headerMessageId,
          threadId: fakeMessage1.threadId,
          draft: true,
          body: '<p>draft</p>',
        });
        this.sessionStub = {
          changes: {
            add: jasmine.createSpy('add'),
          },
        };
        spyOn(DatabaseStore, 'run').andReturn(Promise.resolve([fakeMessage1, this.existingDraft]));
        spyOn(DraftStore, 'sessionForClientId').andReturn(Promise.resolve(this.sessionStub));
      });

      describe('when reply-all is passed', () => {
        it('should add missing participants', async () => {
          this.existingDraft.to = fakeMessage1.participantsForReply().to;
          this.existingDraft.cc = fakeMessage1.participantsForReply().cc;
          const { to, cc } = await DraftFactory.createOrUpdateDraftForReply({
            thread: fakeThread,
            message: fakeMessage1,
            type: 'reply-all',
            behavior: 'prefer-existing',
          });
          expectContactsEqual(to, fakeMessage1.participantsForReplyAll().to);
          expectContactsEqual(cc, fakeMessage1.participantsForReplyAll().cc);
        });

        it('should not blow away other participants who have been added to the draft', async () => {
          const randomA = new Contact({ email: 'other-guy-a@gmail.com' });
          const randomB = new Contact({ email: 'other-guy-b@gmail.com' });
          this.existingDraft.to = fakeMessage1.participantsForReply().to.concat([randomA]);
          this.existingDraft.cc = fakeMessage1.participantsForReply().cc.concat([randomB]);
          const { to, cc } = await DraftFactory.createOrUpdateDraftForReply({
            thread: fakeThread,
            message: fakeMessage1,
            type: 'reply-all',
            behavior: 'prefer-existing',
          });
          expectContactsEqual(to, fakeMessage1.participantsForReplyAll().to.concat([randomA]));
          expectContactsEqual(cc, fakeMessage1.participantsForReplyAll().cc.concat([randomB]));
        });
      });

      describe('when reply is passed', () => {
        it('should remove participants present in the reply-all participant set and not in the reply set', async () => {
          this.existingDraft.to = fakeMessage1.participantsForReplyAll().to;
          this.existingDraft.cc = fakeMessage1.participantsForReplyAll().cc;
          const { to, cc } = await DraftFactory.createOrUpdateDraftForReply({
            thread: fakeThread,
            message: fakeMessage1,
            type: 'reply',
            behavior: 'prefer-existing',
          });
          expectContactsEqual(to, fakeMessage1.participantsForReply().to);
          expectContactsEqual(cc, fakeMessage1.participantsForReply().cc);
        });

        it('should not blow away other participants who have been added to the draft', async () => {
          const randomA = new Contact({ email: 'other-guy-a@gmail.com' });
          const randomB = new Contact({ email: 'other-guy-b@gmail.com' });
          this.existingDraft.to = fakeMessage1.participantsForReplyAll().to.concat([randomA]);
          this.existingDraft.cc = fakeMessage1.participantsForReplyAll().cc.concat([randomB]);
          const { to, cc } = await DraftFactory.createOrUpdateDraftForReply({
            thread: fakeThread,
            message: fakeMessage1,
            type: 'reply',
            behavior: 'prefer-existing',
          });
          expectContactsEqual(to, fakeMessage1.participantsForReply().to.concat([randomA]));
          expectContactsEqual(cc, fakeMessage1.participantsForReply().cc.concat([randomB]));
        });
      });
    });

    describe('when there is not an existing draft at the bottom of the thread', () => {
      beforeEach(() => {
        spyOn(DatabaseStore, 'run').andCallFake(async () => [fakeMessage1]);
        spyOn(DraftFactory, 'createDraftForReply');
      });

      it('should call through to createDraftForReply', async () => {
        await DraftFactory.createOrUpdateDraftForReply({
          thread: fakeThread,
          message: fakeMessage1,
          type: 'reply-all',
          behavior: 'prefer-existing',
        });
        expect(DraftFactory.createDraftForReply).toHaveBeenCalledWith({
          thread: fakeThread,
          message: fakeMessage1,
          type: 'reply-all',
        });

        await DraftFactory.createOrUpdateDraftForReply({
          thread: fakeThread,
          message: fakeMessage1,
          type: 'reply',
          behavior: 'prefer-existing',
        });
        expect(DraftFactory.createDraftForReply).toHaveBeenCalledWith({
          thread: fakeThread,
          message: fakeMessage1,
          type: 'reply',
        });
      });
    });
  });

  // `_current` is typed as EmptyMailboxPerspective from its initializer; `_setPerspective` is
  // the untyped production path, so cast rather than widen to `any`.
  function setFocusedPerspective(perspective: MailboxPerspective) {
    const store = FocusedPerspectiveStore as unknown as { _current: MailboxPerspective };
    store._current = perspective;
  }

  describe('_accountForNewDraft', () => {
    let secondAccount = null;

    beforeEach(() => {
      secondAccount = AccountStore.accounts()[1];
    });

    afterEach(() => {
      // master-before-each never resets FocusedContentStore; keep thread focus from leaking.
      FocusedContentStore._focused = {};
      FocusedContentStore._keyboardCursor = {};
    });

    it('uses the perspective account when the perspective is single-account, ignoring any focused thread from another account', async () => {
      const otherAccountThread = new Thread({
        id: 'other-account-thread-id',
        accountId: secondAccount.id,
        subject: 'Other Account Thread',
      });
      setFocusedPerspective(new MailboxPerspective([account.id]));
      Actions.setFocus({ collection: 'thread', item: otherAccountThread });

      const draft = await DraftFactory.createDraft();
      expect(draft.accountId).toEqual(account.id);
      expect(draft.from[0].email).toEqual(account.defaultMe().email);
      const domain = account.defaultMe().email.split('@')[1];
      expect(draft.headerMessageId.endsWith(`@${domain}`)).toBe(true);
    });

    it("falls back to the perspective's first account when the perspective is multi-account and no thread is focused", async () => {
      setFocusedPerspective(new MailboxPerspective([account.id, secondAccount.id]));

      const draft = await DraftFactory.createDraft();
      expect(draft.accountId).toEqual(account.id);
      expect(draft.from[0].email).toEqual(account.defaultMe().email);
    });

    it('prefers the focused thread account over the perspective order when the perspective is multi-account', async () => {
      const secondAccountThread = new Thread({
        id: 'second-account-thread-id',
        accountId: secondAccount.id,
        subject: 'Second Account Thread',
      });
      setFocusedPerspective(new MailboxPerspective([account.id, secondAccount.id]));
      Actions.setFocus({ collection: 'thread', item: secondAccountThread });

      const draft = await DraftFactory.createDraft();
      expect(draft.accountId).toEqual(secondAccount.id);
      expect(draft.from[0].email).toEqual(secondAccount.defaultMe().email);
    });

    it('prefers the focused thread account regardless of account ordering in the perspective (mirror case)', async () => {
      setFocusedPerspective(new MailboxPerspective([secondAccount.id, account.id]));
      Actions.setFocus({ collection: 'thread', item: fakeThread });

      const draft = await DraftFactory.createDraft();
      expect(draft.accountId).toEqual(account.id);
      expect(draft.from[0].email).toEqual(account.defaultMe().email);
    });

    it("falls back to the perspective's first account, without throwing, when the focused thread's account is not in the perspective", async () => {
      const staleThread = new Thread({
        id: 'stale-cross-account-thread-id',
        accountId: 'account-id-not-in-perspective',
        subject: 'Stale Cross-Account Thread',
      });
      setFocusedPerspective(new MailboxPerspective([account.id, secondAccount.id]));
      Actions.setFocus({ collection: 'thread', item: staleThread });

      const draft = await DraftFactory.createDraft();
      expect(draft.accountId).toEqual(account.id);
      expect(draft.from[0].email).toEqual(account.defaultMe().email);
    });

    it('always uses the pinned core.sending.defaultAccountIdForSend account, regardless of perspective or focused thread', async () => {
      AppEnv.config.set('core.sending.defaultAccountIdForSend', secondAccount.id);
      setFocusedPerspective(new MailboxPerspective([account.id]));

      const draft = await DraftFactory.createDraft();
      expect(draft.accountId).toEqual(secondAccount.id);
      expect(draft.from[0].email).toEqual(secondAccount.defaultMe().email);
    });
  });
});

describe('generateHeaderMessageId', () => {
  const UUID = '[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}';

  it("uses the domain of the sender's address", () => {
    expect(generateHeaderMessageId('ben@example.co.uk')).toMatch(
      new RegExp(`^${UUID}@example\\.co\\.uk$`)
    );
  });

  it('uses the part after the last @', () => {
    expect(generateHeaderMessageId('"a@b"@example.com')).toMatch(
      new RegExp(`^${UUID}@example\\.com$`)
    );
  });

  it('uses the ASCII form of an internationalised domain', () => {
    spyOn(os, 'hostname').andReturn('my-mac.local');
    expect(generateHeaderMessageId('user@münchen.de')).toMatch(
      new RegExp(`^${UUID}@xn--mnchen-3ya\\.de$`)
    );
  });

  it("falls back to the machine's hostname without a usable domain", () => {
    spyOn(os, 'hostname').andReturn('my-mac.local');
    expect(generateHeaderMessageId(undefined)).toMatch(new RegExp(`^${UUID}@my-mac\\.local$`));
    expect(generateHeaderMessageId('nobody')).toMatch(new RegExp(`^${UUID}@my-mac\\.local$`));
    expect(generateHeaderMessageId('x@bad domain')).toMatch(new RegExp(`^${UUID}@my-mac\\.local$`));
  });

  it('falls back to localhost without a hostname', () => {
    spyOn(os, 'hostname').andReturn('');
    expect(generateHeaderMessageId('')).toMatch(new RegExp(`^${UUID}@localhost$`));
  });
});
