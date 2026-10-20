import { ipcRenderer } from 'electron';
import BaseStore from 'mailspring-store';
import { DraftEditingSession } from './editing-session';
import DraftFactory, { ReplyType, ReplyBehavior } from './factory';
import DatabaseStore from '../database/store';
import { SendActionsStore } from '../stores/send-actions';
import FocusedContentStore from '../stores/focused-content';
import { SyncbackDraftTask } from '../tasks/syncback-draft';
import { SyncbackMetadataTask } from '../tasks/syncback-metadata';
import { SendDraftTask } from '../tasks/send-draft';
import { DestroyDraftTask } from '../tasks/destroy-draft';
import { Thread } from '../models/thread';
import { Contact } from '../models/contact';
import { Message } from '../models/message';
import * as Actions from '../actions';
import TaskQueue from '../stores/task-queue';
import MessageBodyProcessor from '../../mail/body/processor';
import SoundRegistry from '../../packages/sound-registry';
import * as ExtensionRegistry from '../../packages/extension-registry';
import { localized } from '../../intl/localize';
import { DatabaseChangeRecord } from '../database/change-record';
import {
  ENGINE_PREPARED_EXTENSIONS,
  bodiesFromSendTask,
  draftPipelineAvailable,
  isEngineBuilt,
  sendDraft as engineSendDraft,
} from '../../engine/client/drafts';

interface IThreadMessageModelOrId {
  thread?: Thread;
  threadId?: string;
  message?: Message;
  messageId?: string;
}

const { DefaultSendActionKey } = SendActionsStore;
/*
Public: DraftStore responds to Actions that interact with Drafts and exposes
public getter methods to return Draft objects and sessions.

It also creates and queues {Task} objects to persist changes through the engine.

Remember that a "Draft" is actually just a "Message" with `draft: true`.

Section: Drafts
*/
class DraftStore extends BaseStore {
  _draftSessions: { [headerMessageId: string]: DraftEditingSession } = {};
  _draftsSending: { [headerMessageId: string]: boolean } = {};

  constructor() {
    super();
    this.listenTo(DatabaseStore, this._onDataChanged);
    this.listenTo(Actions.composeReply, this._onComposeReply);
    this.listenTo(Actions.composeForward, this._onComposeForward);
    this.listenTo(Actions.composeAndSendForward, this._onComposeAndSendForward);
    this.listenTo(Actions.composePopoutDraft, this._onPopoutDraft);
    this.listenTo(Actions.composeNewBlankDraft, this._onPopoutBlankDraft);
    this.listenTo(Actions.composeNewDraftToRecipient, this._onPopoutNewDraftToRecipient);
    this.listenTo(Actions.draftDeliveryFailed, this._onSendDraftFailed);
    this.listenTo(Actions.draftDeliverySucceeded, this._onSendDraftSuccess);
    this.listenTo(Actions.sendQuickReply, this._onSendQuickReply);

    if (AppEnv.isMainWindow()) {
      ipcRenderer.on('new-message', () => {
        Actions.composeNewBlankDraft();
      });
    }

    // Remember that these two actions only fire in the current window and
    // are picked up by the instance of the DraftStore in the current
    // window.
    this.listenTo(Actions.sendDraft, this._onSendDraft);
    this.listenTo(Actions.destroyDraft, this._onDestroyDraft);

    AppEnv.onBeforeUnload(this._onBeforeUnload);

    ipcRenderer.on('mailto', this._onHandleMailtoLink);
    ipcRenderer.on('mailfiles', this._onHandleMailFiles);

    setInterval(
      () => {
        // Slate is unable to properly clear it's caches, so we help it out
        // by flushing them periodically. We care about this a lot because
        // the app is on the same "web page" forever.
        require('slate').resetMemoization();
      },
      5 * 60 * 1000
    ); // 5 min
  }

  /**
  Fetch a {DraftEditingSession} for displaying and/or editing the
  draft with `headerMessageId`.

  @param {String} headerMessageId - The headerMessageId of the draft.
  @returns {Promise} - Resolves to an {DraftEditingSession} for the draft once it has been prepared
  */
  async sessionForClientId(headerMessageId: string) {
    if (!headerMessageId) {
      throw new Error('DraftStore::sessionForClientId requires a headerMessageId');
    }
    if (!this._draftSessions[headerMessageId]) {
      this._draftSessions[headerMessageId] = this._createSession(headerMessageId);
    }
    await this._draftSessions[headerMessageId].prepare();
    return this._draftSessions[headerMessageId];
  }

  // Public: Look up the sending state of the given draft headerMessageId.
  // In popout windows the existance of the window is the sending state.
  isSendingDraft(headerMessageId: string) {
    return this._draftsSending[headerMessageId] || false;
  }

  _doneWithSession(session: DraftEditingSession) {
    session.teardown();
    delete this._draftSessions[session.headerMessageId];
  }

  _cleanupAllSessions() {
    Object.values(this._draftSessions).forEach((session) => {
      this._doneWithSession(session);
    });
  }

  _onBeforeUnload = (readyToUnload: () => void) => {
    const promises = [];

    // Normally we'd just append all promises, even the ones already
    // fulfilled (nothing to save), but in this case we only want to
    // block window closing if we have to do real work. Calling
    // window.close() within on onbeforeunload could do weird things.
    Object.values(this._draftSessions).forEach((session) => {
      const draft = session.draft();
      if (!draft || !draft.id) {
        return;
      }

      // Only delete pristine drafts if we're in popout composers. From the
      // main window we can't know if the draft session may also be open in
      // a popout and have content there.
      if (draft.pristine && !AppEnv.isMainWindow()) {
        Actions.queueTask(
          new DestroyDraftTask({
            messageIds: [draft.id],
            accountId: draft.accountId,
          })
        );
      } else if (session.changes.isDirty()) {
        promises.push(session.changes.commit());
      }
    });

    if (promises.length > 0) {
      let done = () => {
        done = null;
        this._draftSessions = {};
        // We have to wait for accumulateAndTrigger() in the DatabaseStore to
        // send events to ActionBridge before closing the window.
        setTimeout(readyToUnload, 15);
      };

      // Stop and wait before closing, but never wait for more than 700ms.
      // We may not be able to save the draft once the main window has closed
      // and the mailsync bridge is unavailable, don't want to hang forever.
      setTimeout(() => {
        if (done) done();
      }, 700);
      Promise.all(promises).then(() => {
        if (done) done();
      });
      return false;
    }

    // Continue closing
    return true;
  };

  _onDataChanged = (change: DatabaseChangeRecord<Message>) => {
    if (change.objectClass !== Message.name) {
      return;
    }
    const drafts = change.objects.filter((msg) => msg.draft);
    if (drafts.length === 0) {
      return;
    }

    // if the user has canceled an undo send, ensure we no longer show "sending..."
    // this is a fake status!
    for (const draft of drafts) {
      if (this._draftsSending[draft.headerMessageId]) {
        const m = draft.metadataForPluginId('send-later') as any;
        if (m && m.isUndoSend && !m.expiration) {
          delete this._draftsSending[draft.headerMessageId];
        }
      }
    }

    // allow draft editing sessions to update
    this.trigger(change);
  };

  _onSendQuickReply = async (context: IThreadMessageModelOrId, body: string) => {
    if (AppEnv.config.get('core.sending.sounds')) {
      SoundRegistry.playSound('hit-send');
    }

    const resolved = await this._modelifyContext(context);
    if (!resolved.message || !resolved.thread) return;

    const draft = await DraftFactory.createDraftForReply({ ...resolved, type: 'reply' });
    draft.body = `${body}\n\n${draft.body}`;
    draft.pristine = false;

    const t = new SyncbackDraftTask({ draft });
    Actions.queueTask(t);
    await TaskQueue.waitForPerformLocal(t);
    Actions.sendDraft(draft.headerMessageId);
  };

  _onComposeReply = async ({
    thread,
    threadId,
    message,
    messageId,
    popout,
    type,
    behavior,
  }: IThreadMessageModelOrId & {
    popout?: boolean;
    type: ReplyType;
    behavior: ReplyBehavior;
  }) => {
    const resolved = await this._modelifyContext({ thread, threadId, message, messageId });
    if (!resolved.message || !resolved.thread) return;
    const draft = await DraftFactory.createOrUpdateDraftForReply({ ...resolved, type, behavior });
    return this._finalizeAndPersistNewMessage(draft, { popout });
  };

  _onComposeForward = async ({
    thread,
    threadId,
    message,
    messageId,
    popout,
  }: IThreadMessageModelOrId & { popout?: boolean }) => {
    return await this._composeForward({
      thread: thread,
      threadId: threadId,
      message: message,
      messageId: messageId,
      popout: popout,
    });
  };

  _onComposeAndSendForward = async ({
    thread,
    threadId,
    message,
    messageId,
    to,
  }: IThreadMessageModelOrId & { to?: Contact[] }) => {
    const { headerMessageId, draft } = await this._composeForward(
      {
        thread: thread,
        threadId: threadId,
        message: message,
        messageId: messageId,
      },
      to
    );
    Actions.sendDraft(headerMessageId);
  };

  _composeForward = async (
    {
      thread,
      threadId,
      message,
      messageId,
      popout,
    }: IThreadMessageModelOrId & { popout?: boolean },
    to?: Contact[]
  ) => {
    const resolved = await this._modelifyContext({ thread, threadId, message, messageId });
    if (!resolved.message || !resolved.thread) return;
    const draft = await DraftFactory.createDraftForForward(resolved);
    if (to) {
      draft.to = to;
    }
    return this._finalizeAndPersistNewMessage(draft, { popout });
  };

  _modelifyContext({
    thread,
    threadId,
    message,
    messageId,
  }: IThreadMessageModelOrId): Promise<{ thread: Thread; message: Message }> {
    const queries: {
      thread?: Thread | { then: (next: any) => Promise<Thread> };
      message?: Message | { then: (next: any) => Promise<Message> };
    } = {};

    if (thread) {
      if (!(thread instanceof Thread)) {
        throw new Error(
          'newMessageWithContext: `thread` present, expected a Model. Maybe you wanted to pass `threadId`?'
        );
      }
      queries.thread = thread;
    } else if (threadId && threadId.length) {
      queries.thread = DatabaseStore.find<Thread>(Thread, threadId);
    } else {
      throw new Error('newMessageWithContext: `thread` or `threadId` is required.');
    }

    if (message) {
      if (!(message instanceof Message)) {
        throw new Error(
          'newMessageWithContext: `message` present, expected a Model. Maybe you wanted to pass `messageId`?'
        );
      }
      // MessageStore loads the bodies of expanded messages only.
      queries.message =
        message.body !== undefined
          ? message
          : DatabaseStore.find<Message>(Message, message.id)
              .include(Message.attributes.body)
              .then((loaded) => loaded || message);
    } else if (messageId && messageId.length) {
      queries.message = DatabaseStore.find<Message>(Message, messageId).include(
        Message.attributes.body
      );
    } else {
      // Reply to the newest received/sent message, never to an unsent draft on the thread.
      // No limit: hidden messages (reminders) must not push the real one out of the window, so
      // the scan skips bodies and the chosen message's body is loaded on its own.
      queries.message = DatabaseStore.findAll<Message>(Message, { threadId: threadId || thread.id })
        .where(Message.attributes.draft.equal(false))
        .order(Message.attributes.date.descending())
        .then((messages) => {
          const latest = messages.find((m) => !m.isHidden());
          return latest
            ? DatabaseStore.find<Message>(Message, latest.id).include(Message.attributes.body)
            : null;
        });
    }

    return Promise.props(queries);
  }

  async _finalizeAndPersistNewMessage(draft: Message, { popout }: { popout?: boolean } = {}) {
    // Give extensions an opportunity to perform additional setup to the draft. A draft the
    // engine built has the built-in ones' work done already.
    const engineBuilt = isEngineBuilt(draft);
    ExtensionRegistry.Composer.extensions().forEach((extension) => {
      if (!extension.prepareNewDraft) {
        return;
      }
      if (engineBuilt && ENGINE_PREPARED_EXTENSIONS.includes(extension.name)) {
        return;
      }
      extension.prepareNewDraft({ draft });
    });

    // Optimistically create a draft session and hand it the draft so that it
    // doesn't need to do a query for it a second from now when the composer wants it.
    this._createSession(draft.headerMessageId, draft);

    const task = new SyncbackDraftTask({ draft });
    Actions.queueTask(task);

    await TaskQueue.waitForPerformLocal(task);
    if (popout) {
      this._onPopoutDraft(draft.headerMessageId);
    }

    // In Playwright E2E tests, mailsync is not running so the draft is never
    // persisted and no database delta is emitted. Emit a synthetic change record
    // so that MessageStore picks up the draft and renders the inline composer.
    if (process.env.PLAYWRIGHT && !popout) {
      // Assign a temporary id if mailsync hasn't provided one, so that
      // Message.isHidden() and other code that reads `id` doesn't throw.
      if (!draft.id) {
        draft.id = `draft-${draft.headerMessageId}`;
      }
      DatabaseStore.trigger(
        new DatabaseChangeRecord({
          type: 'persist',
          objectClass: Message.name,
          objects: [draft],
          objectsRawJSON: [draft.toJSON()],
        })
      );
    }

    return { headerMessageId: draft.headerMessageId, draft };
  }

  _createSession(headerMessageId: string, draft?: Message) {
    this._draftSessions[headerMessageId] = new DraftEditingSession(headerMessageId, draft);
    return this._draftSessions[headerMessageId];
  }

  _onPopoutNewDraftToRecipient = async (contact: Contact) => {
    const draft = await DraftFactory.createNewDraft({ to: [contact] });
    await this._finalizeAndPersistNewMessage(draft, { popout: true });
  };

  _onPopoutBlankDraft = async () => {
    const draft = await DraftFactory.createNewDraft();
    const { headerMessageId } = await this._finalizeAndPersistNewMessage(draft);
    await this._onPopoutDraft(headerMessageId, { newDraft: true });
  };

  _onPopoutDraft = async (headerMessageId: string, options: { newDraft?: boolean } = {}) => {
    if (headerMessageId == null) {
      throw new Error('DraftStore::onPopoutDraftId - You must provide a headerMessageId');
    }

    const session = await this.sessionForClientId(headerMessageId);
    if (!session.draft()) return;

    await session.changes.commit();
    const draftJSON = session.draft().toJSON();

    // Since we pass a windowKey, if the popout composer draft already
    // exists we'll simply show that one instead of spawning a whole new
    // window.
    AppEnv.newWindow({
      hidden: true, // We manually show in ComposerWithWindowProps::onDraftReady
      windowType: 'composer',
      windowKey: `composer-${headerMessageId}`,
      windowProps: Object.assign(options, { headerMessageId, draftJSON }),
      title: options.newDraft ? localized('New Message') : localized('Message'),
    });
  };

  _onHandleMailtoLink = async (event: Electron.IpcRendererEvent, urlString: string) => {
    // returned promise is just used for specs
    const draft = await DraftFactory.createDraftForMailto(urlString);
    try {
      await this._finalizeAndPersistNewMessage(draft, { popout: true });
    } catch (err) {
      AppEnv.showErrorDialog(err.toString());
    }
  };

  _onHandleMailFiles = async (event: Electron.IpcRendererEvent, paths: string[]) => {
    // returned promise is just used for specs
    const draft = await DraftFactory.createNewDraft();
    const { headerMessageId } = await this._finalizeAndPersistNewMessage(draft);

    let remaining = paths.length;
    const callback = () => {
      remaining -= 1;
      if (remaining === 0) {
        this._onPopoutDraft(headerMessageId);
      }
    };

    paths.forEach((path) => {
      Actions.addAttachment({
        filePath: path,
        headerMessageId: headerMessageId,
        onCreated: callback,
      });
    });
  };

  _onDestroyDraft = ({
    accountId,
    headerMessageId,
    id,
  }: {
    accountId: string;
    headerMessageId: string;
    id?: string;
  }) => {
    const session = this._draftSessions[headerMessageId];

    // Immediately reset any pending changes so no saves occur
    if (session) {
      this._doneWithSession(session);
    }

    // Stop any pending tasks related to the draft
    TaskQueue.queue().forEach((task) => {
      if (task instanceof SyncbackDraftTask && task.headerMessageId === headerMessageId) {
        Actions.cancelTask(task);
      }
      if (task instanceof SendDraftTask && task.headerMessageId === headerMessageId) {
        Actions.cancelTask(task);
      }
    });

    // Queue the task to destroy the draft
    if (id) {
      Actions.queueTask(new DestroyDraftTask({ accountId, messageIds: [id] }));
    } else {
      console.warn('Tried to delete a draft that had no ID assigned yet.');
    }

    // In Playwright E2E tests, emit a synthetic unpersist so MessageStore
    // removes the draft from its items and the inline composer disappears.
    // Only do this on the first destroy call (when the session existed) to
    // avoid a race where a duplicate destroy triggers _fetchFromCache and
    // overwrites items that include a newly created draft.
    if (process.env.PLAYWRIGHT && id && session) {
      const draft = new Message({ id, accountId, headerMessageId, draft: true } as any);
      DatabaseStore.trigger(
        new DatabaseChangeRecord({
          type: 'unpersist',
          objectClass: Message.name,
          objects: [draft],
          objectsRawJSON: [],
        })
      );
    }

    if (AppEnv.isComposerWindow()) {
      AppEnv.close();
    }
  };

  _onSendDraft = async (
    headerMessageId: string,
    options: { delay?: number; actionKey?: string } = {}
  ) => {
    const { delay = AppEnv.config.get('core.sending.undoSend'), actionKey = DefaultSendActionKey } =
      options;

    this._draftsSending[headerMessageId] = true;

    const sendAction = SendActionsStore.sendActionForKey(actionKey);
    if (!sendAction) {
      throw new Error(`Cant find send action ${actionKey} `);
    }
    // The engine sends it (draft.send) and keeps its undo window, unless the action is a
    // plugin's (src/engine/client/drafts.ts).
    const viaEngine =
      draftPipelineAvailable() && SendActionsStore.enginePerformsSendAction(actionKey);

    const sendLaterMetadataValue: {
      expiration: Date;
      isUndoSend: boolean;
      actionKey: string;
      clientFinalize?: boolean;
    } | null =
      delay > 0
        ? { expiration: new Date(Date.now() + delay), isUndoSend: true, actionKey: actionKey }
        : null;

    // get the draft session, apply any last-minute edits and get the final draft.
    // We need to call `changes.commit` here to ensure the body of the draft is
    // completely saved and the user won't see old content briefly.
    const session = await this.sessionForClientId(headerMessageId);

    // Collect diagnostic context as we proceed so we can attach it to any error
    // report if the draft is not found at the end. This helps us understand which
    // code path was taken without adding overhead to the happy path.
    const diagnostics: Record<string, unknown> = {
      sessionExisted: !!this._draftSessions[headerMessageId],
      draftIdBeforeEnsure: session.draft()?.id,
      draftAccountIdBeforeEnsure: session.draft()?.accountId,
    };

    // move the draft to another account if necessary to match the from: field
    const accountIdBeforeEnsure = session.draft()?.accountId;
    try {
      await session.ensureCorrectAccount();
    } catch (err) {
      // Clear the sending flag, or the composer treats the draft as mid-send and
      // ignores every later click of Send.
      this._draftsSending[headerMessageId] = false;
      this.trigger({ headerMessageId });
      if (!session.draft()) {
        return this._onUnexpectedNotFoundDuringSend(headerMessageId, {
          ...diagnostics,
          failedAt: 'session.draft() was null in ensureCorrectAccount',
        });
      }
      if (session.hasSendableFromAddress()) {
        AppEnv.showErrorDialog(localized('Sorry, this message could not be sent. %@', err.message));
        AppEnv.reportError(err, { headerMessageId });
      } else {
        AppEnv.showErrorDialog(DraftEditingSession.unsendableFromAddressMessage());
      }
      return;
    }
    diagnostics.ensureCorrectAccountChangedAccount =
      session.draft()?.accountId !== accountIdBeforeEnsure;
    diagnostics.draftIdAfterEnsure = session.draft()?.id;
    diagnostics.draftAccountIdAfterEnsure = session.draft()?.accountId;

    let draft: Message = session.draft();
    if (!draft) {
      this._draftsSending[headerMessageId] = false;
      this.trigger({ headerMessageId });
      return this._onUnexpectedNotFoundDuringSend(headerMessageId, {
        ...diagnostics,
        failedAt: 'session.draft() returned null after ensureCorrectAccount',
      });
    }

    // remove inline attachments that are no longer in the body
    const files = draft.files.filter((f) => {
      return !(f.contentId && !draft.body.includes(`cid:${f.contentId}`));
    });
    if (files.length !== draft.files.length) {
      session.changes.add({ files });
    }

    // attach send-later metadata if a send delay is enabled
    if (sendLaterMetadataValue && !viaEngine) {
      if (SendActionsStore.needsClientFinalize(draft, actionKey)) {
        sendLaterMetadataValue.clientFinalize = true;
      }
      session.changes.addPluginMetadata('send-later', sendLaterMetadataValue);
    }

    diagnostics.dirtyFieldsBeforeCommit = session.changes.dirtyFields();
    diagnostics.commitPromiseInFlight = !!(session.changes as any)._commitPromise;
    await session.changes.commit();
    diagnostics.dirtyFieldsAfterCommit = session.changes.dirtyFields();
    await session.teardown();

    // ensureCorrectAccount / commit may assign this draft a new ID. To move forward
    // we need to have the final object with it's final ID.
    draft = await DatabaseStore.findBy<Message>(Message, { headerMessageId, draft: true }).include(
      Message.attributes.body
    );
    if (!draft) {
      this._draftsSending[headerMessageId] = false;
      this.trigger({ headerMessageId });
      return this._onUnexpectedNotFoundDuringSend(headerMessageId, {
        ...diagnostics,
        failedAt: 'DatabaseStore.findBy returned null after commit',
      });
    }

    // Directly update the message body cache so the user immediately sees
    // the new message text (and never old draft text or blank text) sending.
    await MessageBodyProcessor.updateCacheForMessage(draft);

    // At this point the message UI enters the sending state and the composer is unmounted.
    this.trigger({ headerMessageId });
    this._doneWithSession(session);

    if (viaEngine) {
      await this._sendWithEngine(draft, { delay, actionKey });
      if (AppEnv.isComposerWindow()) {
        AppEnv.close();
      }
      return;
    }

    // To be able to undo the send, we need to pretend that we added the send-later
    // metadata as it's own task so that the undo action is clear. We don't actually
    // want a separate SyncbackMetadataTask to be queued because a stray SyncbackDraftTask
    // could overwrite the metadata value back to null.
    if (sendLaterMetadataValue) {
      Actions.queueUndoOnlyTask(
        SyncbackMetadataTask.forSaving({
          pluginId: 'send-later',
          model: draft,
          value: sendLaterMetadataValue,
          undoValue: { expiration: null, isUndoSend: true },
        })
      );
    } else {
      // Immediately send the draft
      await sendAction.performSendAction({ draft });
    }

    if (AppEnv.isComposerWindow()) {
      AppEnv.close();
    }
  };

  // draft.send with this window's send transforms as `bodies`. With a delay the engine writes
  // the send-later metadata; the Undo Send entry (queued in the main window, which may not be
  // this one) calls draft.cancelSend (UndoRedoStore, engine/client/drafts.ts undoSendTarget).
  async _sendWithEngine(
    draft: Message,
    { delay, actionKey }: { delay: number; actionKey: string }
  ) {
    const { headerMessageId, accountId } = draft;
    let result: { scheduledAt?: number };
    try {
      const prepared = SendDraftTask.forSending(draft);
      const bodies = bodiesFromSendTask(prepared);
      const transformed =
        (bodies.perRecipient && Object.keys(bodies.perRecipient).length > 0) ||
        bodies.self !== draft.body;
      result = await engineSendDraft({
        accountId,
        headerMessageId,
        delayMs: delay > 0 ? delay : undefined,
        after: actionKey,
        bodies: transformed ? bodies : undefined,
      });
    } catch (err) {
      this._draftsSending[headerMessageId] = false;
      this.trigger({ headerMessageId });
      const why = (err.data && err.data.message) || err.message;
      AppEnv.showErrorDialog(localized('Sorry, this message could not be sent. %@', why));
      return;
    }
    if (!result || !result.scheduledAt) return;

    const value = {
      expiration: new Date(result.scheduledAt * 1000),
      isUndoSend: true,
      actionKey,
    };
    Actions.queueUndoOnlyTask(
      SyncbackMetadataTask.forSaving({
        pluginId: 'send-later',
        model: draft,
        value,
        undoValue: { expiration: null, isUndoSend: true },
      })
    );
  }

  _onUnexpectedNotFoundDuringSend = (
    headerMessageId?: string,
    diagnostics?: Record<string, unknown>
  ) => {
    const msg = localized(
      'Sorry, the draft you tried to send could not be found. Please try again.'
    );
    AppEnv.showErrorDialog(msg);
    const err = new Error('Could not find draft after finalizing session for sending.');
    // Pass diagnostics as `extra`, which reportError logs next to the error.
    AppEnv.reportError(err, { headerMessageId, ...diagnostics });
  };

  _onSendDraftSuccess = ({ headerMessageId }) => {
    delete this._draftsSending[headerMessageId];
    this.trigger({ headerMessageId });
  };

  _onSendDraftFailed = ({ headerMessageId, threadId, errorMessage, errorDetail }) => {
    this._draftsSending[headerMessageId] = false;
    this.trigger({ headerMessageId });

    if (AppEnv.isMainWindow()) {
      // We delay so the view has time to update the restored draft. If we
      // don't delay the modal may come up in a state where the draft looks
      // like it hasn't been restored or has been lost.
      //
      // We also need to delay because the old draft window needs to fully
      // close. It takes windows currently (June 2016) 100ms to close by
      setTimeout(() => {
        const focusedThread = FocusedContentStore.focused('thread');
        if (threadId && focusedThread && focusedThread.id === threadId) {
          AppEnv.showErrorDialog(errorMessage, { detail: errorDetail });
        } else {
          Actions.composePopoutDraft(headerMessageId, { errorMessage, errorDetail });
        }
      }, 300);
    }
  };
}

export default new DraftStore();
