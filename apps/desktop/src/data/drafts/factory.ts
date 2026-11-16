import os from 'os';
import { domainToASCII } from 'url';
import * as Actions from '../actions';
import DatabaseStore from '../database/store';
import { AccountStore } from '../stores/account';
import ContactStore from '../stores/contact';
import { MessageStore } from '../stores/message';
import FocusedPerspectiveStore from '../stores/focused-perspective';
import FocusedContentStore from '../stores/focused-content';
import { Contact } from '../models/contact';
import { Message } from '../models/message';
import { Thread } from '../models/thread';
import { wrapPlaintext } from '../../components/composer-editor/plaintext';
import { buildDraft } from '../../engine/client/drafts';
import { DraftCreateParams } from '../../engine/protocol';
import { lazyRequire } from '../lazy-require';

const DraftStore = lazyRequire((): typeof import('./store').default => require('./store').default);

// Message-IDs are `<UUID>@<domain of the sending address>`, so they don't name the client.
// Internationalised domains go out in their ASCII (punycode) form. Falls back to the machine's
// hostname when the address has no usable domain.
export function generateHeaderMessageId(fromEmail?: string) {
  const at = (fromEmail || '').lastIndexOf('@');
  let domain = at === -1 ? '' : (fromEmail || '').slice(at + 1).trim();
  // domainToASCII returns '' for anything that is not a valid domain.
  domain = domain && /[^ -~]/.test(domain) ? domainToASCII(domain) : domain;
  if (!/^[A-Za-z0-9.-]+$/.test(domain)) {
    domain = (os.hostname() || '').replace(/[^A-Za-z0-9.-]/g, '') || 'localhost';
  }
  return `${crypto.randomUUID().toUpperCase()}@${domain}`;
}

export type ReplyType = 'reply' | 'reply-all';
export type ReplyBehavior = 'prefer-existing' | 'prefer-existing-if-pristine';

class DraftFactory {
  useHTML() {
    const forcePlaintext = AppEnv.keymaps.getIsAltKeyDown();
    return AppEnv.config.get('core.composing.html') && !forcePlaintext;
  }

  // Builds the draft in the engine (src/engine/client/drafts.ts): quoting, signature,
  // participants and tracker stripping are airpost-compose's. `fields` are Message attributes over
  // what the kind builds.
  _engineDraft(params: DraftCreateParams, fields: { [key: string]: any } = {}): Promise<Message> {
    const json: { [key: string]: unknown } = {};
    for (const [key, value] of Object.entries(fields)) {
      const attr = (Message.attributes as any)[key];
      if (!attr || value === undefined || key === 'accountId') continue;
      json[attr.jsonKey || key] = attr.toJSON(value);
    }
    return buildDraft({ ...params, html: this.useHTML(), fields: json });
  }

  // A new message (the composer's blank draft, mailto aside). DraftStore's compose actions
  // come here; createDraft stays the plain builder other code adds its own content to.
  createNewDraft(fields: { accountId?: string; [key: string]: any } = {}) {
    const account = fields.accountId ? null : this._accountForNewDraft();
    return this._engineDraft({ kind: 'new', accountId: fields.accountId || account?.id }, fields);
  }

  async createDraft(fields: { accountId?: string; [key: string]: any } = {}) {
    // A reply or forward names its account; its auto-cc/bcc is that account's.
    const account =
      (fields.accountId && AccountStore.accountForId(fields.accountId)) ||
      this._accountForNewDraft();
    if (!account) {
      throw new Error('createDraft: there is no account to send from');
    }
    const rich = this.useHTML();
    const defaults = {
      body: rich ? '<br/>' : '',
      subject: '',
      version: 0,
      unread: false,
      starred: false,
      headerMessageId: null as string | null,
      from: [account.defaultMe()],
      date: new Date(),
      draft: true,
      pristine: true,
      plaintext: !rich,
      accountId: account.id,
      cc: [] as Contact[],
      bcc: [] as Contact[],
    };

    const merged = Object.assign(defaults, fields);
    if (!merged.headerMessageId) {
      const from = merged.from && merged.from[0];
      merged.headerMessageId = generateHeaderMessageId(from && from.email);
    }

    const autoContacts = await ContactStore.parseContactsInString(account.autoaddress.value);
    if (account.autoaddress.type === 'cc') {
      merged.cc = (merged.cc || []).concat(autoContacts);
    }
    if (account.autoaddress.type === 'bcc') {
      merged.bcc = (merged.bcc || []).concat(autoContacts);
    }
    if (merged.plaintext) {
      merged.body = wrapPlaintext(merged.body);
    }

    return new Message(merged);
  }

  createDraftForMailto(urlString: string) {
    const account = this._accountForNewDraft();
    return this._engineDraft({
      kind: 'mailto',
      accountId: account && account.id,
      mailto: urlString,
    });
  }

  async createOrUpdateDraftForReply({
    message,
    thread,
    type,
    behavior,
  }: {
    message: Message;
    thread: Thread;
    type: ReplyType;
    behavior: ReplyBehavior;
  }) {
    if (!['reply', 'reply-all'].includes(type)) {
      throw new Error(`createOrUpdateDraftForReply called with ${type}, not reply or reply-all`);
    }

    const existingDraft = await this.candidateDraftForUpdating(message, behavior);
    if (existingDraft) {
      return this.updateDraftForReply(existingDraft, { message, type });
    }
    return this.createDraftForReply({ message, thread, type });
  }

  createDraftForReply({
    message,
    thread,
    type,
  }: {
    message: Message;
    thread: Thread;
    type: ReplyType;
  }) {
    return this._engineDraft(
      { kind: type, accountId: message.accountId, messageId: message.id },
      { threadId: thread.id }
    );
  }

  createDraftForForward({ thread, message }: { thread: Thread; message: Message }) {
    // Start downloading the attachments, if they haven't been already
    message.files.forEach((f) => Actions.fetchFile(f));
    return this._engineDraft(
      { kind: 'forward', accountId: message.accountId, messageId: message.id },
      { threadId: thread.id }
    );
  }

  async createDraftForSendAgain(message: Message) {
    // Reuse the same attachment preparation path as forwarding. This ensures files are
    // available locally when the duplicated draft is eventually sent.
    message.files.forEach((file) => Actions.fetchFile(file));

    const draft = await this.createDraft({
      to: [...message.to],
      cc: [...message.cc],
      bcc: [...message.bcc],
      from: [...message.from],
      subject: message.subject,
      files: [...message.files],
      accountId: message.accountId,
    });

    // Assign these after createDraft so its new-message formatting defaults do not alter
    // the original content. This is intentionally a new message with no reply / forward
    // metadata or thread association.
    draft.body = message.body || (message.plaintext ? '' : '<br/>');
    draft.plaintext = message.plaintext;
    return draft;
  }

  // Deprecated since plugin API 1.37.0 and removed in 2.0.0: the engine quotes the message in
  // the drafts it builds (createDraftForReply, createDraftForForward).
  prepareBodyForQuoting(_message: Message): Promise<string> {
    throw new Error(
      'DraftFactory.prepareBodyForQuoting is gone: createDraftForReply and createDraftForForward quote the message'
    );
  }

  // Deprecated since plugin API 1.37.0 and removed in 2.0.0: the engine's expiration executor
  // sends the resurfacing messages of snoozes and reminders.
  createDraftForResurfacing(_thread: Thread, _threadMessageId: string | null, _body: string) {
    throw new Error(
      'DraftFactory.createDraftForResurfacing is gone: the engine sends resurfacing messages itself'
    );
  }

  // The thread's unsent reply drafts, the ones a reply should reopen last: those answering
  // `message` sort after those answering an older message, newest last in each. A thread
  // keeps one reply draft, as in Gmail, so Reply on a newer message reopens the draft the
  // user started on an older one.
  async replyDraftsInThread(message: Message) {
    const messages =
      message.threadId === MessageStore.threadId()
        ? MessageStore.items()
        : await DatabaseStore.findAll<Message>(Message, { threadId: message.threadId });

    const drafts = messages
      .filter(
        (other) =>
          other.draft === true &&
          other.accountId === message.accountId &&
          !!other.replyToHeaderMessageId &&
          !other.forwardedHeaderMessageId &&
          !other.isHidden()
      )
      .sort((a, b) => (a.date ? a.date.getTime() : 0) - (b.date ? b.date.getTime() : 0));
    const answersMessage = (d: Message) => d.replyToHeaderMessageId === message.headerMessageId;
    return [...drafts.filter((d) => !answersMessage(d)), ...drafts.filter(answersMessage)];
  }

  async candidateDraftForUpdating(message: Message, behavior: ReplyBehavior) {
    if (!['prefer-existing-if-pristine', 'prefer-existing'].includes(behavior)) {
      return null;
    }

    const candidateDrafts = await this.replyDraftsInThread(message);
    if (candidateDrafts.length === 0) {
      return null;
    }
    if (behavior === 'prefer-existing') {
      const draft = candidateDrafts[candidateDrafts.length - 1];
      // MessageStore holds bodies for expanded messages only, and the read above skips them;
      // the draft is persisted again with its body, so load it first.
      if (draft.body !== undefined) return draft;
      const withBody = await DatabaseStore.find<Message>(Message, draft.id).include(
        Message.attributes.body
      );
      return withBody || null;
    }
    if (behavior === 'prefer-existing-if-pristine') {
      const sessions = await Promise.all(
        candidateDrafts.map((candidateDraft) =>
          DraftStore().sessionForClientId(candidateDraft.headerMessageId)
        )
      );
      const pristine = sessions.map((s) => s.draft()).filter((d) => d && d.pristine);
      return pristine[pristine.length - 1];
    }
  }

  updateDraftForReply(draft: Message, { type, message }: { type: ReplyType; message: Message }) {
    if (!(message && draft)) {
      throw new Error('updateDraftForReply: Expected message and existing draft.');
    }
    const { to, cc } = this.participantsForReplyUpdate(draft, { type, message });
    draft.to = to;
    draft.cc = cc;
    return draft;
  }

  // The draft's To and Cc after switching it to a reply or a reply-all to `message`, keeping
  // the recipients the user added.
  participantsForReplyUpdate(
    draft: Message,
    { type, message }: { type: ReplyType; message: Message }
  ) {
    const updated: { to: Contact[]; cc: Contact[] } = { to: [...draft.to], cc: [...draft.cc] };
    const replySet = message.participantsForReply();
    const replyAllSet = message.participantsForReplyAll();
    let targetSet: { to: Contact[]; cc: Contact[] };

    if (type === 'reply') {
      targetSet = replySet;

      // Remove participants present in the reply-all set and not the reply set
      for (const key of ['to', 'cc'] as const) {
        updated[key] = updated[key].filter((contact) => {
          const inReplySet = replySet[key]?.find((x) => x.email === contact.email);
          const inReplyAllSet = replyAllSet[key]?.find((x) => x.email === contact.email);
          return !(inReplyAllSet && !inReplySet);
        });
      }
    } else {
      // Add participants present in the reply-all set and not on the draft
      // Switching to reply-all shouldn't really ever remove anyone.
      targetSet = replyAllSet;
    }

    for (const key of ['to', 'cc'] as const) {
      for (const contact of targetSet[key]) {
        if (!updated[key]?.find((x) => x.email === contact.email)) {
          updated[key].push(contact);
        }
      }
    }

    return updated;
  }

  _accountForNewDraft() {
    const defAccountId = AppEnv.config.get('core.sending.defaultAccountIdForSend');
    const account = AccountStore.accountForId(defAccountId);
    if (account) return account;

    const perspectiveAccountIds = FocusedPerspectiveStore.current().accountIds;

    if (perspectiveAccountIds.length > 1) {
      const focusedThread = FocusedContentStore.focused('thread');
      if (focusedThread && perspectiveAccountIds.includes(focusedThread.accountId)) {
        const focusedAccount = AccountStore.accountForId(focusedThread.accountId);
        if (focusedAccount) return focusedAccount;
      }
    }

    const focusedAccountId = perspectiveAccountIds[0];
    if (focusedAccountId) return AccountStore.accountForId(focusedAccountId);
    return AccountStore.accounts()[0];
  }
}

export default new DraftFactory();
