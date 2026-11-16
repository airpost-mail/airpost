import React from 'react';
import ReactDOM from 'react-dom';
import { localized } from '../../intl/localize';
import * as Utils from '../../data/models/utils';
import * as Actions from '../../data/actions';
import DraftStore from '../../data/drafts/store';
import { DraftEditingSession, MessageWithEditorState } from '../../data/drafts/editing-session';
import * as DragDropTypes from '../../components/drag-drop-types';
import * as EmlUtils from '../../mail/eml';
import { File } from '../../data/models/file';
import { webUtils } from 'electron';
import path from 'path';
import { DropZone } from '../../components/drop-zone';
import { RetinaImg } from '../../components/retina-img';
import { ScrollRegion } from '../../components/scroll-region';
import { TabGroupRegion } from '../../components/tab-group-region';
import { KeyCommandsRegion } from '../../components/key-commands-region';
import { InjectedComponentSet } from '../../components/injected/component-set';
import { ComposerEditor } from '../../components/composer-editor/composer-editor';
import { ComposerEditorPlaintext } from '../../components/composer-editor/plaintext-editor';
import * as ComposerSupport from '../../components/composer-editor/support';
import { RovingTabIndexToolbar } from '../../components/roving-tab-index-toolbar';
import { ComposerHeader } from './header';
import { SendActionButton } from './send-action-button';
import { ActionBarPlugins } from './action-bar-plugins';
import { AttachmentsArea } from './attachments-area';
import { QuotedTextControl } from './quoted-text-control';
import Fields from './fields';
import { FilesScheme, resolveFilesSchemeURL } from '../../mail/files-scheme';
import { IconButton } from '../../components/kit/icon-button';
import { Dialogs } from '../../platform/app-api';

const { hasBlockquote, hasNonTrailingBlockquote, hideQuotedTextByDefault } =
  ComposerSupport.BaseBlockPlugins;

interface ComposerViewProps {
  draft: MessageWithEditorState;
  session: DraftEditingSession;
  className?: string;
  // Set by previews whose list keeps keyboard focus while the selection moves.
  skipInitialFocus?: boolean;
}
interface ComposerViewState {
  quotedTextHidden: boolean;
  quotedTextPresent: boolean;
  isDropping: boolean;
  attachingThreadCount: number;
}

// The disk path behind a drag from inside the app: an attachment chip or an image in a message.
export function nonNativeFilePathForDrop(dataTransfer: DataTransfer): string | null {
  if (dataTransfer.types.includes('text/airpost-file-url')) {
    const downloadURL = dataTransfer.getData('text/airpost-file-url');
    const downloadFilePath = downloadURL.split('file://')[1];
    if (downloadFilePath) {
      return downloadFilePath;
    }
  }

  // Accept drops of images from within the app
  if (dataTransfer.types.includes('text/uri-list')) {
    const uri = dataTransfer.getData('text/uri-list');
    if (uri.indexOf('file://') === 0) {
      return decodeURI(uri.split('file://')[1]);
    }
    // Inline images in a message are airpost-file: URLs (src/mail/files-scheme.ts).
    if (uri.startsWith(`${FilesScheme}:`)) {
      const first = uri.split(/\r?\n/)[0].trim();
      return resolveFilesSchemeURL(path.join(AppEnv.getConfigDirPath(), 'files'), first);
    }
  }
  return null;
}

// The ComposerView is a unique React component because it (currently) is a
// singleton. Normally, the React way to do things would be to re-render the
// Composer with new props.
export default class ComposerView extends React.Component<ComposerViewProps, ComposerViewState> {
  static displayName = 'ComposerView';

  _mounted = false;
  // A send check is waiting for the engine or a dialog.
  _validating = false;
  _mouseDownTarget: HTMLElement | null = null;

  dropzone = React.createRef<DropZone>();
  sendButton = React.createRef<SendActionButton>();
  focusContainer = React.createRef<KeyCommandsRegion & HTMLDivElement>();
  editor: React.RefObject<ComposerEditorPlaintext> | React.RefObject<ComposerEditor> =
    React.createRef<any>();
  header = React.createRef<ComposerHeader>();

  _keymapHandlers = {
    'composer:send-message': () => this.sendButton.current?.primarySend(),
    'composer:show-and-focus-bcc': () => this.header.current?.showAndFocusField(Fields.Bcc),
    'composer:show-and-focus-cc': () => this.header.current?.showAndFocusField(Fields.Cc),
    'composer:focus-to': () => this.header.current?.showAndFocusField(Fields.To),
    'composer:show-and-focus-from': () => {},
    'composer:select-attachment': () => this._onSelectAttachment(),
    'composer:delete-empty-draft': (e: Event) => {
      this.props.draft.pristine && this._onDestroyDraft();
      e.preventDefault();
    },
  };

  constructor(props: ComposerViewProps) {
    super(props);

    // props.draft is the session's draft when the decorator rendered us.
    const draft = props.session.draft() ?? props.draft;

    this.state = {
      isDropping: false,
      attachingThreadCount: 0,
      quotedTextPresent: !!hasBlockquote(draft.bodyEditorState),
      quotedTextHidden: hideQuotedTextByDefault(draft),
    };
  }

  componentDidMount() {
    const { date, files } = this.props.draft;

    this._mounted = true;

    files.forEach((file) => {
      if (Utils.shouldDisplayAsImage(file)) {
        Actions.fetchFile(file);
      }
    });

    // Note: the way this is implemented, `date` updates each time the draft is saved,
    // so this will autoselect the draft if it has been edited or created in the last 3s.
    const isBrandNew = Date.now() - (date instanceof Date ? date.getTime() : Number(date)) < 3000;
    if (isBrandNew && !this.props.skipInitialFocus) {
      const el = ReactDOM.findDOMNode(this) as HTMLElement;
      window.requestAnimationFrame(() => {
        el.scrollIntoView(false);
        this.focus();
      });
    }
  }

  componentDidUpdate() {
    const { draft } = this.props;

    // If the user has added an inline blockquote, show all the quoted text
    // note: this is necessary because it's hidden with CSS that can't be
    // made more specific.
    if (this.state.quotedTextHidden && hasNonTrailingBlockquote(draft.bodyEditorState)) {
      this.setState({ quotedTextHidden: false });
    }
  }

  componentWillUnmount() {
    this._mounted = false;
  }

  focus() {
    if (!this._mounted || !this.header.current || !this.editor.current) return;

    // If something within us already has focus, don't change it. Never, ever
    // want to pull the cursor out from under the user while typing
    const node = ReactDOM.findDOMNode(this.focusContainer.current);
    if (node && node.contains(document.activeElement)) {
      return;
    }

    if (this.props.draft.to.length === 0 || this.props.draft.subject.length === 0) {
      this.header.current.focus();
    } else {
      this.editor.current.focus();
    }
  }

  _renderContent() {
    const restrictWidth = AppEnv.config.get('core.reading.restrictMaxWidth');
    const { quotedTextHidden, quotedTextPresent } = this.state;
    const { draft, session } = this.props;

    return (
      <div className={`composer-centered ${restrictWidth && 'restrict-width'}`}>
        <ComposerHeader ref={this.header} draft={draft} session={session} />
        <div
          className="compose-body"
          onMouseUp={this._onMouseUpComposerBody}
          onMouseDown={this._onMouseDownComposerBody}
        >
          <div className="composer-body-wrap">
            {draft.plaintext ? (
              <ComposerEditorPlaintext
                ref={this.editor as React.RefObject<ComposerEditorPlaintext>}
                value={draft.body}
                propsForPlugins={{ draft, session }}
                onFileReceived={this._onFileReceived}
                onDrop={(e) => this.dropzone.current?._onDrop(e as React.DragEvent<HTMLDivElement>)}
                onChange={(body) => {
                  session.changes.add({ body });
                }}
              />
            ) : (
              <>
                <ComposerEditor
                  ref={this.editor as React.RefObject<ComposerEditor>}
                  value={draft.bodyEditorState}
                  className={quotedTextHidden ? 'hiding-quoted-text' : undefined}
                  propsForPlugins={{ draft, session }}
                  onFileReceived={this._onFileReceived}
                  onUpdatedSlateEditor={(editor) => session.setMountedEditor(editor)}
                  onDrop={(e) =>
                    this.dropzone.current?._onDrop(e as React.DragEvent<HTMLDivElement>)
                  }
                  onChange={(change) => {
                    // We minimize thrashing and support editors in multiple windows by ensuring
                    // non-value changes (eg focus) to the editorState don't trigger database saves
                    const skipSaving =
                      change.operations.size > 0 &&
                      change.operations.every(
                        (op) =>
                          !!op &&
                          (op.type === 'set_selection' ||
                            (op.type === 'set_value' &&
                              Object.keys(op.properties).every((k) => k === 'decorations')))
                      );
                    session.changes.add({ bodyEditorState: change.value }, { skipSaving });
                  }}
                />
                <QuotedTextControl
                  quotedTextHidden={quotedTextHidden}
                  quotedTextPresent={quotedTextPresent}
                  onUnhide={() => this.setState({ quotedTextHidden: false })}
                  onRemove={() => {
                    this.setState({ quotedTextHidden: false }, () =>
                      this.editor.current?.removeQuotedText()
                    );
                  }}
                />
              </>
            )}

            <AttachmentsArea draft={draft} attaching={this.state.attachingThreadCount > 0} />
          </div>
          <div className="composer-footer-region">
            <InjectedComponentSet
              deferred
              direction="column"
              matching={{ role: 'Composer:Footer' }}
              exposedProps={{
                draft: draft,
                threadId: draft.threadId,
                headerMessageId: draft.headerMessageId,
                session: session,
              }}
            />
          </div>
        </div>
      </div>
    );
  }

  // This lets us click outside of the `contenteditable`'s `contentBody`
  // and simulate what happens when you click beneath the text *in* the
  // contentEditable.

  // Unfortunately, we need to manually keep track of the "click" in
  // separate mouseDown, mouseUp events because we need to ensure that the
  // start and end target are both not in the contenteditable. This ensures
  // that this behavior doesn't interfear with a click and drag selection.
  _onMouseDownComposerBody = (event: React.MouseEvent<HTMLDivElement>) => {
    const editorNode = ReactDOM.findDOMNode(this.editor.current);
    if (editorNode && editorNode.contains(event.target as Node)) {
      this._mouseDownTarget = null;
    } else {
      this._mouseDownTarget = event.target as HTMLElement;
    }
  };

  _inFooterRegion(el: HTMLElement) {
    return el.closest && el.closest('.composer-footer-region');
  }

  _onMouseUpComposerBody = (event: React.MouseEvent<HTMLDivElement>) => {
    if (
      event.target === this._mouseDownTarget &&
      !this._inFooterRegion(event.target as HTMLElement)
    ) {
      // We don't set state directly here because we want the native
      // contenteditable focus behavior. When the contenteditable gets focused
      const editor = this.editor.current;
      const editorNode = ReactDOM.findDOMNode(editor) as HTMLElement | null;
      if (editor && editorNode) {
        const bodyRect = editorNode.getBoundingClientRect();

        if (event.pageY < bodyRect.top) {
          editor.focus();
        } else {
          if (this.state.quotedTextHidden) {
            editor.focusEndReplyText();
          } else {
            editor.focusEndAbsolute();
          }
        }
      }
    }
    this._mouseDownTarget = null;
  };

  _shouldAcceptDrop = (event: React.DragEvent<HTMLDivElement>) => {
    // If the drag was initiated within Slate, let Slate handle it by falling through
    // and not preventing default. Slate can drag-drop image (void) nodes on it's own.
    if (event.dataTransfer.types.includes('application/x-slate-fragment')) {
      return false;
    }

    // Ensure that you can't pick up a file and drop it on the same draft
    const nonNativeFilePath = this._nonNativeFilePathForDrop(event);
    const hasNativeFile = event.dataTransfer.types.includes('Files');
    const hasNonNativeFilePath = nonNativeFilePath !== null;

    return hasNativeFile || hasNonNativeFilePath || this._hasThreadsForDrop(event);
  };

  _hasThreadsForDrop = (event: React.DragEvent<HTMLDivElement>) => {
    return event.dataTransfer.types.includes(DragDropTypes.ThreadsDragType);
  };

  _nonNativeFilePathForDrop = (event: React.DragEvent<HTMLDivElement>) => {
    return nonNativeFilePathForDrop(event.dataTransfer);
  };

  _onDrop = (event: React.DragEvent<HTMLDivElement>) => {
    // Accept drops of real files from other applications
    for (const file of Array.from(event.dataTransfer.files)) {
      this._onFileReceived(webUtils.getPathForFile(file));
      event.preventDefault();
    }

    // Accept drops from attachment components / images within the app
    const uri = this._nonNativeFilePathForDrop(event);
    if (uri) {
      this._onFileReceived(uri);
      event.preventDefault();
    }

    // dataTransfer is only valid for the duration of this handler, so read the
    // payload now and hand the ids off to an async worker.
    if (this._hasThreadsForDrop(event)) {
      this._onThreadsReceived(event.dataTransfer.getData(DragDropTypes.ThreadsDragType));
      event.preventDefault();
    }
  };

  _onThreadsReceived = async (json: string) => {
    let threadIds: string[] = [];
    try {
      threadIds = JSON.parse(json).threadIds || [];
    } catch (err) {
      return;
    }
    if (!threadIds.length) {
      return;
    }

    // Fetching the raw message from the sync engine is a remote round trip, so
    // show a placeholder in the attachments area until the files land.
    const dropCount = threadIds.length;
    this.setState((state) => ({ attachingThreadCount: state.attachingThreadCount + dropCount }));

    let staged: Array<{ filePath: string }> = [];
    let unavailableThreadIds: string[] = [];
    try {
      ({ staged, unavailableThreadIds } = await EmlUtils.stageThreadsAsEml(threadIds));
    } catch (err) {
      AppEnv.reportError(err);
    } finally {
      if (this._mounted) {
        this.setState((state) => ({
          attachingThreadCount: state.attachingThreadCount - dropCount,
        }));
      }
    }

    if (!this._mounted) {
      return;
    }
    for (const { filePath } of staged) {
      Actions.addAttachment({
        filePath,
        headerMessageId: this.props.draft.headerMessageId,
        // The attachment store copies the file into its own directory before
        // this fires, so the staged copy is free to go.
        onCreated: () => EmlUtils.discardStagedEml(filePath),
      });
    }

    // A thread with nothing exportable in it and a fetch that failed are
    // different problems, and only the second one is worth retrying.
    const problems = [];
    if (unavailableThreadIds.length) {
      problems.push(
        localized('One or more of the conversations have no message that can be attached.')
      );
    }
    if (staged.length < threadIds.length - unavailableThreadIds.length) {
      problems.push(
        localized('One or more of the original messages could not be downloaded. Please try again.')
      );
    }
    if (problems.length) {
      AppEnv.showErrorDialog(problems.join('\n\n'));
    }
  };

  _onFileReceived = (filePath: string) => {
    // called from onDrop and onFilePaste - assume images should be inline
    Actions.addAttachment({
      filePath: filePath,
      headerMessageId: this.props.draft.headerMessageId,
      onCreated: (file: File) => {
        if (!this._mounted) return;
        if (this.props.draft.plaintext) return;

        if (Utils.shouldDisplayAsImage(file)) {
          const { draft, session } = this.props;
          const match = draft.files.find((f) => f.id === file.id);
          if (!match) {
            return;
          }
          match.contentId = Utils.generateContentId();
          session.changes.add({
            files: [...draft.files],
          });

          this.editor.current?.insertInlineAttachment(file);
        }
      },
    });
  };

  // Resolves true when the draft can go: the engine's checks (draft.validate) found no errors
  // and the user answered its warnings. Action bar plugins get it as the `isValidDraft` prop.
  _isValidDraft = async (): Promise<boolean> => {
    // We need to check the `DraftStore` because the `DraftStore` is
    // immediately and synchronously updated as soon as this function
    // fires. Since `setState` is asynchronous, if we used that as our only
    // check, then we might get a false reading.
    if (this._validating || DraftStore.isSendingDraft(this.props.draft.headerMessageId)) {
      return false;
    }
    this._validating = true;
    try {
      return await this._validate();
    } catch (err) {
      AppEnv.showErrorDialog(String((err as Error).message || err));
      return false;
    } finally {
      this._validating = false;
    }
  };

  _validate = async (): Promise<boolean> => {
    const { session } = this.props;
    const { recipientErrors, recipientWarnings, miscErrors, miscWarnings } =
      await session.validateForSending();

    // Display errors
    const errors = [...recipientErrors, ...miscErrors];
    if (errors.length > 0) {
      await Dialogs.showMessageBox(
        {
          type: 'warning',
          buttons: [localized('Edit Message'), localized('Cancel')],
          message: localized('Cannot send message'),
          detail: errors[0],
        },
        'none'
      );
      return false;
    }

    // Display warnings
    if (recipientWarnings.length > 0) {
      const response = await Dialogs.ask(
        {
          type: 'warning',
          buttons: [
            localized('Send Anyway'),
            localized('Send & Ignore Warnings For This Email'),
            localized('Cancel'),
          ],
          message: localized('Are you sure?'),
          detail: recipientWarnings.join('. '),
        },
        'none'
      );
      if (response === 1) {
        // Send & Ignore Future Warnings for Recipient Email
        session.addRecipientsToWarningBlacklist();
      } else if (response !== 0) {
        return false;
      }
    }
    if (miscWarnings.length > 0) {
      const response = await Dialogs.ask(
        {
          type: 'warning',
          buttons: [localized('Send Anyway'), localized('Cancel')],
          message: localized('Are you sure?'),
          detail: miscWarnings.join('. '),
        },
        'none'
      );
      if (response !== 0) {
        return false;
      }
    }

    return true;
  };

  _onDestroyDraft = () => {
    Actions.destroyDraft(this.props.draft);
  };

  _onSelectAttachment = () => {
    Actions.selectAttachment({ headerMessageId: this.props.draft.headerMessageId });
  };

  render() {
    return (
      <div className={this.props.className}>
        <KeyCommandsRegion
          localHandlers={this._keymapHandlers}
          className={'message-item-white-wrap composer-outer-wrap'}
          ref={this.focusContainer}
          tabIndex={-1}
        >
          <TabGroupRegion className="composer-inner-wrap">
            <DropZone
              ref={this.dropzone}
              className="composer-inner-wrap"
              shouldAcceptDrop={this._shouldAcceptDrop}
              onDragStateChange={({ isDropping }) => this.setState({ isDropping })}
              onDrop={this._onDrop}
            >
              <DropToAttachCover visible={this.state.isDropping} />

              <div className="composer-content-wrap">
                {AppEnv.isComposerWindow() ? (
                  <ScrollRegion className="compose-body-scroll">
                    {this._renderContent()}
                  </ScrollRegion>
                ) : (
                  this._renderContent()
                )}
              </div>

              <div className="composer-action-bar-workspace-wrap">
                <InjectedComponentSet
                  deferred
                  matching={{ role: 'Composer:ActionBarWorkspace' }}
                  exposedProps={{
                    draft: this.props.draft,
                    threadId: this.props.draft.threadId,
                    headerMessageId: this.props.draft.headerMessageId,
                    session: this.props.session,
                  }}
                />
              </div>

              <footer className="composer-action-bar-wrap" data-tooltips-anchor>
                <div className="tooltips-container" />
                <RovingTabIndexToolbar
                  label={localized('Composer Actions')}
                  className="composer-action-bar-content"
                >
                  <ActionBarPlugins
                    draft={this.props.draft}
                    session={this.props.session}
                    isValidDraft={this._isValidDraft}
                  />
                  <DeleteButton onClick={this._onDestroyDraft} />
                  <AttachFileButton onClick={this._onSelectAttachment} />

                  <div style={{ order: 0, flex: 1 }} />

                  <SendActionButton
                    tabIndex={-1}
                    ref={this.sendButton}
                    style={{ order: -100 }}
                    draft={this.props.draft}
                    isValidDraft={this._isValidDraft}
                  />
                </RovingTabIndexToolbar>
              </footer>
            </DropZone>
          </TabGroupRegion>
        </KeyCommandsRegion>
      </div>
    );
  }
}

const DropToAttachCover = (props: { visible: boolean }) => (
  <div className="composer-drop-cover" style={{ display: props.visible ? 'block' : 'none' }}>
    <div className="centered">
      <RetinaImg name="composer-drop-to-attach.png" mode={RetinaImg.Mode.ContentIsMask} />
      {localized(`Drop to Attach`)}
    </div>
  </div>
);

const AttachFileButton = (props: { onClick: () => void }) => (
  <IconButton
    variant="toolbar"
    tabIndex={-1}
    className="btn-attach"
    style={{ order: 0 }}
    icon="icon-composer-attachment.png"
    label={localized('Attach File')}
    onClick={props.onClick}
  />
);

const DeleteButton = (props: { onClick: () => void }) => (
  <IconButton
    variant="toolbar"
    tabIndex={-1}
    className="btn-trash"
    style={{ order: 100 }}
    icon="icon-composer-trash.png"
    label={localized('Delete Draft')}
    onClick={props.onClick}
  />
);
// Note: tabIndex={-1} on individual buttons is intentional - the RovingTabIndexToolbar
// wrapper manages which button has tabIndex={0} at any given time.
