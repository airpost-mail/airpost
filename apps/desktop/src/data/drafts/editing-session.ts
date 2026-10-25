import * as Immutable from 'immutable';
import BaseStore from '../stores/base';
import { Editor, Value, Block, Text } from 'slate';

import { localized } from '../../intl/localize';

import TaskQueue from '../stores/task-queue';
import { Message } from '../models/message';
import * as Utils from '../models/utils';
import { validateDraft } from '../../engine/client/drafts';
import type { DraftProblem } from '../../engine/protocol';
import * as Actions from '../actions';
import DatabaseStore from '../database/store';
import { AccountStore } from '../stores/account';
import { DraftChangeSet } from './change-set';
import { DatabaseChangeRecord } from '../database/change-record';
import { DestroyDraftTask } from '../tasks/destroy-draft';
import { Composer as ComposerExtensionRegistry } from '../../packages/extension-registry';
import { SyncbackDraftTask } from '../tasks/syncback-draft';
import { attributeValue, setAttributeValue } from '../models/attribute-access';
import { lazyRequire } from '../lazy-require';

// The session loads drafts with their body, and the hotwired getter keeps it a string.
export type MessageWithEditorState = Message & { body: string; bodyEditorState: any };

/*
Note: This is a bit of a hack, but pulling in composer-editor/support is what triggers slate,
slate-react, etc. to be loaded in windows where the components are not actually in use.
By lazily resolving this import, we can save ~400ms of window start-up time.
*/
const Conversion = lazyRequire(
  (): typeof import('../../components/composer-editor/support').Conversion =>
    require('../../components/composer-editor/support').Conversion
);

function convertFromHTML(html: string) {
  return Conversion().convertFromHTML(html);
}
function convertToHTML(value: Value) {
  return Conversion().convertToHTML(value);
}
function convertToShapeWithoutContent(value: Value) {
  return Conversion().convertToShapeWithoutContent(value);
}

const MetadataChangePrefix = 'metadata.';

export interface DraftValidation {
  recipientErrors: string[];
  recipientWarnings: string[];
  miscErrors: string[];
  miscWarnings: string[];
}

// The recipients the user chose "Send & Ignore Warnings For This Email" for.
function warningBlacklist(): string[] {
  // JSON.parse(null) parses "null", as a missing key always has.
  const list = JSON.parse(localStorage.getItem('recipientWarningBlacklist') ?? 'null');
  return Array.isArray(list) ? list : [];
}

// A draft.validate finding in the app's language; the engine's text is English.
function problemText(p: DraftProblem): string {
  switch (p.code) {
    case 'no-recipients':
      return localized('You need to provide one or more recipients before sending the message.');
    case 'salutation-mismatch': {
      const name = /\("(.*)"\)$/.exec(p.message);
      return name
        ? localized(
            `The message is addressed to a name that doesn't appear to be a recipient ("%@")`,
            name[1]
          )
        : p.message;
    }
    case 'unsendable-from':
      return DraftEditingSession.unsendableFromAddressMessage();
    case 'blank-subject':
      return localized('The subject field is blank.');
    case 'attachment-mentioned':
      return localized('The message mentions an attachment but none are attached.');
    case 'empty-template':
      return localized('The message contains an empty template area.');
    default:
      return p.message;
  }
}
const DraftStore = lazyRequire((): typeof import('./store').default => require('./store').default);

// What DraftStore triggers with: a database change, or a draft's send state changing.
type DraftDatabaseChange = DatabaseChangeRecord<Message> & { headerMessageId?: string };
type DraftStoreChange = DraftDatabaseChange | { headerMessageId: string };

function isPersistChange(change: DraftStoreChange): change is DraftDatabaseChange {
  return 'type' in change && change.type === 'persist';
}

function hotwireDraftBodyState(draft: any, session: DraftEditingSession): MessageWithEditorState {
  // Populate the bodyEditorState and override the draft properties
  // so that they're kept in sync with minimal recomputation.
  let _bodyHTMLValue: string | null = draft.body;
  let _bodyEditorValue: Value | null = null;

  draft.__bodyPropDescriptor = {
    configurable: true,
    get: function () {
      if (_bodyHTMLValue === null) {
        // One of the two values is always set.
        _bodyHTMLValue = convertToHTML(_bodyEditorValue!);
      }
      return _bodyHTMLValue;
    },
    set: function (inHTML: string) {
      if (_bodyHTMLValue === inHTML) return;

      _bodyHTMLValue = inHTML;

      // Capture the editor reference immediately. Between this check and the first use of
      // the editor below (moveToStartOfDocument), convertFromHTML is called. In rare cases
      // the composer can unmount during that call — or during the Slate change operations
      // themselves — setting session._mountedEditor to null. Using a local snapshot prevents
      // the null-reference crash at the point of use.
      const mountedEditor = session._mountedEditor;
      if (mountedEditor) {
        const wasFocused = mountedEditor.value.selection.isFocused;
        const inHTMLEditorValue = convertFromHTML(inHTML);
        try {
          // try to apply the new value to the existing document to preserve undo history.
          let edits = mountedEditor.moveToStartOfDocument();

          // remove all but the very first node in the document
          const [first, ...rest] = edits.value.document.nodes.toArray();
          for (const item of rest) {
            if (edits.value.document.getPath(item.key)) {
              edits = edits.removeNodeByKey(item.key);
            }
          }

          // Note: an empty block must still contain a Text node — a block with zero
          // children leaves the document in a state Slate can't compute a selection
          // range over (moveToRangeOfDocument crashes on Point.moveToStartOfNode).
          edits = edits
            .replaceNodeByKey(
              first.key,
              Block.create({ type: 'div', nodes: Immutable.List([Text.create('')]) })
            )
            .moveToRangeOfDocument()
            .insertFragment(inHTMLEditorValue.document);

          // occasionally inserting the fragment adds a new line at the beginning of the value.
          // It's unclear why this happens and it appears to be specific to replies.
          const firstBlock = edits.value.document.getBlocks().first();
          if (firstBlock && firstBlock.text === '') {
            edits = edits.removeNodeByKey(firstBlock.key);
          }

          // Note: We must re-focus the body or the composer is blurred after this operation.
          // A copy coming in from the engine leaves the focus where it was.
          edits = edits.moveToStart();
          if (wasFocused || !session._applyingIncoming) edits = edits.focus();

          _bodyEditorValue = edits.value;
        } catch (err) {
          // deleting and re-inserting the whole document seems to push Slate pretty hard and it
          // sometimes fails with odd schema issues (undefined node, invalid range.) Just fall
          // back to blowing away undo rather than getting the editor stuck.

          // These errors are "downstream" from the actual problem (an invalid document model).
          // To debug this, we convert the doc to JSON and replace all the actual text / html
          // with "XXXX". Sending this home means we can replay the failing transform with an
          // equivalent document of the same shape.
          AppEnv.reportError(new Error(`Unable to insert fragment into existing document.`), {
            underlyingError: err,
            existingSlateShape: mountedEditor
              ? convertToShapeWithoutContent(mountedEditor.value)
              : null,
            incomingSlateShape: convertToShapeWithoutContent(inHTMLEditorValue),
          });
          _bodyEditorValue = inHTMLEditorValue;
        }
      } else {
        // compute it again when it's asked for
        _bodyEditorValue = null;
      }
    },
  };

  draft.__bodyEditorValuePropDescriptor = {
    configurable: true,
    get: function () {
      if (_bodyEditorValue === null) {
        // One of the two values is always set.
        _bodyEditorValue = convertFromHTML(_bodyHTMLValue!);
      }
      return _bodyEditorValue;
    },
    set: function (inValue: Value) {
      if (_bodyEditorValue === inValue) return;
      _bodyHTMLValue = null;
      _bodyEditorValue = inValue;
    },
  };

  Object.defineProperty(draft, 'body', draft.__bodyPropDescriptor);
  Object.defineProperty(draft, 'bodyEditorState', draft.__bodyEditorValuePropDescriptor);
  draft.body = _bodyHTMLValue;

  return draft as MessageWithEditorState;
}

/**
 * Note: This method is intended to return a new Mesasge object so that lazy people doing
 * shallow equals get the correct basic behavior as the draft is modified in the session.
 *
 * However, this method does not deep clone array values (To:, etc.) and the hot-wired body
 * and bodyEditorValue are linked through the same internal state. Changing the body of
 * the cloned draft changes the body of the old draft too.
 *
 * At the moment these tradeoffs seem OK because we're really just trying to make
 * "props.draft !== nextProps.draft" work.
 */
function fastCloneDraft(draft: MessageWithEditorState) {
  const next = new Message({});
  // Copies every own property, attributes and the hot-wired descriptors alike.
  const source = draft as unknown as Record<string, unknown>;
  const target = next as unknown as Record<string, unknown>;
  for (const key of Object.getOwnPropertyNames(draft)) {
    if (key === 'body' || key === 'bodyEditorState') continue;
    target[key] = source[key];
  }
  Object.defineProperty(next, 'body', (next as any).__bodyPropDescriptor);
  Object.defineProperty(next, 'bodyEditorState', (next as any).__bodyEditorValuePropDescriptor);
  return next as MessageWithEditorState;
}

/**
Public: DraftEditingSession is a small class that makes it easy to implement components
that display Draft objects or allow for interactive editing of Drafts.

1. It synchronously provides an instance of a draft via `draft()`, and
   triggers whenever that draft instance has changed.

2. It provides an interface for modifying the draft that transparently
   batches changes, and ensures that the draft provided via `draft()`
   always has pending changes applied.

Section: Drafts
*/
export class DraftEditingSession extends BaseStore {
  static DraftChangeSet = DraftChangeSet;

  _draft: MessageWithEditorState | null = null;
  _draftPromise: Promise<Message | undefined>;
  _destroyed = false;
  _mountedEditor: Editor | null = null;
  _applyingIncoming = false;

  headerMessageId: string;
  changes = new DraftChangeSet({
    onAddChanges: (changes) => this.changeSetApplyChanges(changes),
    onCommit: () => this.changeSetCommit(), // for specs
  });

  constructor(headerMessageId: string, draft: Message | null = null) {
    super();

    this.headerMessageId = headerMessageId;

    this.listenTo(DraftStore(), this._onDraftChanged);

    this._mountedEditor = null;

    if (draft && draft.body !== undefined) {
      this._draftPromise = Promise.resolve(draft);
      this._draft = hotwireDraftBodyState(draft, this);
    } else {
      this._draftPromise = DatabaseStore.findBy<Message>(Message, {
        headerMessageId: this.headerMessageId,
        draft: true,
      })
        .include(Message.attributes.body)
        .then((draft) => {
          if (this._destroyed) {
            console.warn(`Draft loaded but session has been torn down.`);
            return;
          }
          if (!draft) {
            console.warn(`Draft ${this.headerMessageId} could not be found. Just deleted?`);
            return;
          }
          this._draft = hotwireDraftBodyState(draft, this);
          this.trigger();
          return draft;
        });
    }
  }

  // Public: Returns the draft object with the latest changes applied.
  //
  draft() {
    return this._draft;
  }

  prepare() {
    return this._draftPromise;
  }

  teardown() {
    this.stopListeningToAll();
    this.changes.clearDelayedCommit();
    this._destroyed = true;
    this._mountedEditor = null;
  }

  setMountedEditor(editor: Editor | null) {
    this._mountedEditor = editor;
  }

  // ensureCorrectAccount cannot send from an address that matches no account or
  // alias, eg: a draft synced from another client, or an alias since removed.
  hasSendableFromAddress() {
    const from = this._draft?.from[0];
    return !!(from && AccountStore.accountForEmail(from.email));
  }

  static unsendableFromAddressMessage() {
    return localized(
      'The From address is not one of your accounts or aliases. Choose a From address and try again.'
    );
  }

  // The draft, for the methods that run only once the session has prepared it.
  _loadedDraft() {
    if (!this._draft) {
      throw new Error('DraftEditingSession: the draft is not loaded');
    }
    return this._draft;
  }

  /**
   * The checks before a send: the engine's (`draft.validate`) on the draft as saved, after
   * saving the session's changes, then the v1 plugins' `warningsForSending` hooks. Errors stop
   * the send; warnings ask first.
   */
  async validateForSending(): Promise<DraftValidation> {
    await this.changes.commit();
    const draft = this._loadedDraft();
    const result = await validateDraft({
      accountId: draft.accountId,
      headerMessageId: draft.headerMessageId,
      skipSalutationFor: warningBlacklist(),
    });
    const validation: DraftValidation = {
      recipientErrors: [],
      recipientWarnings: [],
      miscErrors: [],
      miscWarnings: [],
    };
    for (const p of result.errors) {
      (p.group === 'recipients' ? validation.recipientErrors : validation.miscErrors).push(
        problemText(p)
      );
    }
    for (const p of result.warnings) {
      (p.group === 'recipients' ? validation.recipientWarnings : validation.miscWarnings).push(
        problemText(p)
      );
    }
    for (const extension of ComposerExtensionRegistry.extensions()) {
      if (extension.warningsForSending) {
        validation.miscWarnings.push(...extension.warningsForSending({ draft }));
      }
    }
    return validation;
  }

  /** Deprecated since plugin API 1.38.0 and removed in 2.0.0: use `validateForSending()`. */
  validateDraftForSending(): never {
    throw new Error('DraftEditingSession.validateDraftForSending: use validateForSending()');
  }

  /** Deprecated since plugin API 1.38.0 and removed in 2.0.0: use `validateForSending()`. */
  validateDraftRecipients(): never {
    throw new Error('DraftEditingSession.validateDraftRecipients: use validateForSending()');
  }

  addRecipientsToWarningBlacklist() {
    const draft = this._loadedDraft();
    const allRecipients = [...draft.to, ...draft.cc, ...draft.bcc];
    const allRecipientEmails = allRecipients.map((contact) => contact.email);
    // JSON.parse(null) parses "null", as a missing key always has.
    let blacklist = JSON.parse(localStorage.getItem('recipientWarningBlacklist') ?? 'null');
    if (blacklist === null) blacklist = [];
    blacklist.push(...allRecipientEmails);
    localStorage.setItem('recipientWarningBlacklist', JSON.stringify(blacklist));
  }

  checkRecipientInWarningBlacklist(email: string) {
    return warningBlacklist().includes(email);
  }

  // This function makes sure the draft is attached to a valid account, and changes
  // it's accountId if the from address does not match the account for the from
  // address.
  //
  async ensureCorrectAccount() {
    const draft = this._loadedDraft();
    const from = draft.from[0];
    if (!from) {
      throw new Error('DraftEditingSession::ensureCorrectAccount - draft has no from address.');
    }
    const account = AccountStore.accountForEmail(from.email);
    if (!account) {
      throw new Error(
        'DraftEditingSession::ensureCorrectAccount - you can only send drafts from a configured account.'
      );
    }

    if (account.id !== draft.accountId) {
      // Create a new draft in the new account (with a new ID).
      // Because we use the headerMessageId /exclusively/ as the
      // identifier we'll be fine.
      //
      // Then destroy the old one, since it may be synced to the server
      // and require cleanup!
      //
      const create = new SyncbackDraftTask({
        headerMessageId: draft.headerMessageId,
        draft: new Message({
          from: draft.from,
          version: 0,
          to: draft.to,
          cc: draft.cc,
          bcc: draft.bcc,
          body: draft.body,
          files: draft.files,
          replyTo: draft.replyTo,
          subject: draft.subject,
          // Carry over plugin metadata (open/link tracking, send-later) so toggles the
          // user enabled aren't silently reset when the draft moves between accounts.
          pluginMetadata: draft.pluginMetadata,
          headerMessageId: draft.headerMessageId,
          accountId: account.id,
          unread: false,
          starred: false,
          draft: true,
        }),
      });

      const destroy =
        draft.id &&
        new DestroyDraftTask({
          messageIds: [draft.id],
          accountId: draft.accountId,
        });

      Actions.queueTask(create);
      await TaskQueue.waitForPerformLocal(create);
      if (destroy) Actions.queueTask(destroy);
    }

    return this;
  }

  _onDraftChanged = (change?: DraftStoreChange) => {
    if (change === undefined || !isPersistChange(change)) {
      return;
    }
    if (!this._draft) {
      // We don't accept changes unless our draft object is loaded
      return;
    }

    // Some change events just tell us that the state of the draft (eg sending state)
    // have changed and don't include a payload.
    if (change.headerMessageId) {
      if (change.headerMessageId === this._draft.headerMessageId) {
        this.trigger();
      }
      return;
    }

    const draft = this._draft;
    const nextDraft = change.objects
      .filter((obj) => obj.draft && obj.headerMessageId === draft.headerMessageId)
      .pop();

    if (!nextDraft) {
      return;
    }

    // A persist of this draft is a save of ours coming back, or another device's revision
    // the engine adopted (docs/drafts-sync-design.md section 6), which can also move the row
    // to a new id under the same headerMessageId. The session takes its values except in the
    // fields the user edited since the last save, so it never replaces what they're writing.
    const lockedFields = this.changes.lockedFields();
    const incoming: [string, unknown][] = [];
    for (const key of Object.keys(Message.attributes)) {
      if (key === 'headerMessageId' || lockedFields.includes(key)) continue;
      const value = attributeValue(nextDraft, key);
      if (value === undefined || value === null) continue;
      if (Utils.isEqual(attributeValue(draft, key), value)) continue;
      incoming.push([key, value]);
    }
    if (incoming.length === 0) {
      return;
    }

    const next = fastCloneDraft(draft);
    this._draft = next;
    this._applyingIncoming = true;
    try {
      for (const [key, value] of incoming) setAttributeValue(next, key, value);
    } finally {
      this._applyingIncoming = false;
    }
    this.trigger();
  };

  async changeSetCommit() {
    if (this._destroyed || !this._draft) {
      return;
    }
    const task = new SyncbackDraftTask({ draft: this._draft });
    Actions.queueTask(task);
    await TaskQueue.waitForPerformLocal(task);
  }

  changeSetApplyChanges = (changes: Record<string, unknown>) => {
    if (this._destroyed) {
      return;
    }
    if (!this._draft) {
      throw new Error('DraftChangeSet was modified before the draft was prepared.');
    }

    const draft = fastCloneDraft(this._draft);
    this._draft = draft;

    for (const [key, val] of Object.entries(changes)) {
      if (key.startsWith(MetadataChangePrefix)) {
        const parts = key.split(MetadataChangePrefix);
        draft.directlyAttachMetadata(parts[parts.length - 1], val as Record<string, any>);
      } else {
        setAttributeValue(draft, key, val);
      }
    }
    this.trigger();
  };
}
