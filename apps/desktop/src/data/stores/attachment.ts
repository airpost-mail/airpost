import os from 'os';
import _fs from 'fs';
import path from 'path';
import { shell } from 'electron';
import BaseStore from './base';
import DraftStore from '../drafts/store';
import * as Actions from '../actions';
import { File } from '../models/file';
import * as Utils from '../models/utils';
import { localized } from '../../intl/localize';
import { Dialogs } from '../../platform/app-api';
import { filesSchemeURL } from '../../mail/files-scheme';
import EngineEvents from '../../engine/client/events';
import { ErrorCode, FilesGetResult, Method } from '../../engine/protocol';
import { confirmOpeningFile, markFileFromInternet } from '../../mail/attachment-safety';
import {
  generatePreview,
  canPossiblyPreviewExtension,
  displayQuickPreviewWindow,
} from '../../mail/quick-preview';

Promise.promisifyAll(_fs);
const fs = _fs as any;

const fileAccessibleAtPath = async (filePath: string) => {
  try {
    await fs.accessAsync(filePath, fs.F_OK);
    return true;
  } catch (ex) {
    return false;
  }
};

export type AttachmentDownloadData = null;

class AttachmentStore extends BaseStore {
  _filePreviewPaths: { [fileId: string]: string } = {};
  _filesDirectory: string = path.join(AppEnv.getConfigDirPath(), 'files');
  _lastDownloadDirectory?: string;
  _pendingAddsByDraft: { [headerMessageId: string]: Promise<void> } = {};

  constructor() {
    super();

    // viewing messages
    this.listenTo(Actions.fetchFile, this._fetch);
    this.listenTo(Actions.fetchAndOpenFile, this._fetchAndOpen);
    this.listenTo(Actions.fetchAndSaveFile, this._fetchAndSave);
    this.listenTo(Actions.fetchAndSaveAllFiles, this._fetchAndSaveAll);
    this.listenTo(Actions.quickPreviewFile, this._quickPreviewFile);

    // sending
    this.listenTo(Actions.addAttachment, this._onAddAttachment);
    this.listenTo(Actions.selectAttachment, this._onSelectAttachment);
    this.listenTo(Actions.removeAttachment, this._onRemoveAttachment);

    fs.mkdirSync(this._filesDirectory, { recursive: true });
  }

  // Returns a path on disk for saving the file. Note that we must account
  // for files that don't have a name and avoid returning <downloads/dir/"">
  // which causes operations to happen on the directory (badness!)
  //
  pathForFile(file: File): string;
  pathForFile(file: File | null | undefined): string | null;
  pathForFile(file: File | null | undefined) {
    if (!file) {
      return null;
    }
    const id = file.id.toLowerCase();
    const filePath = path.join(
      this._filesDirectory,
      id.substr(0, 2),
      id.substr(2, 2),
      id,
      file.safeDisplayName()
    );
    if (!filePath.startsWith(this._filesDirectory + path.sep)) {
      return null;
    }
    return filePath;
  }

  // pathForFile for the store's own reads and writes, which can't go on without a path.
  _requirePathForFile(file: File) {
    const filePath = this.pathForFile(file);
    if (!filePath) {
      throw new Error(`${file && file.id} has no path inside the files directory`);
    }
    return filePath;
  }

  // The airpost-file: URL message documents use to show the file (src/mail/files-scheme.ts).
  fileSchemeURLForFile(file: File) {
    const filePath = this.pathForFile(file);
    return filePath ? filesSchemeURL(this._filesDirectory, filePath) : null;
  }

  getDownloadDataForFile(fileId: string): AttachmentDownloadData {
    // if we ever support downloads again, put this back
    return null;
  }

  // Returns a hash of download objects keyed by fileId
  getDownloadDataForFiles(fileIds: string[] = []) {
    const downloadData: { [fileId: string]: AttachmentDownloadData } = {};
    fileIds.forEach((fileId) => {
      downloadData[fileId] = this.getDownloadDataForFile(fileId);
    });
    return downloadData;
  }

  previewPathsForFiles(fileIds: string[] = []) {
    const previewPaths: { [fileId: string]: string } = {};
    fileIds.forEach((fileId) => {
      previewPaths[fileId] = this.previewPathForFile(fileId);
    });
    return previewPaths;
  }

  previewPathForFile(fileId: string): string {
    return this._filePreviewPaths[fileId];
  }

  // Public: Resolve a file's on-disk path. The engine finds it (files.get), fetching the
  // message's body first when the file isn't there yet.
  resolvePathForFile(file: File): Promise<string> {
    return this._prepareAndResolveFilePath(file);
  }

  async _prepareAndResolveFilePath(file: File) {
    const filePath = await this._enginePathForFile(file);
    if (await fileAccessibleAtPath(filePath)) {
      this._ensurePreviewOfFile(file, filePath);
    }
    return filePath;
  }

  // A file the engine doesn't store yet (one just attached to a draft, without an account) is
  // where the store copied it.
  async _enginePathForFile(file: File): Promise<string> {
    if (!file.accountId) return this._requirePathForFile(file);
    let result: FilesGetResult;
    try {
      result = await EngineEvents.request<FilesGetResult>(Method.FilesGet, {
        accountId: file.accountId,
        fileId: file.id,
      });
    } catch (err) {
      if ((err as { code?: number }).code === ErrorCode.NotFound) {
        return this._requirePathForFile(file);
      }
      throw err;
    }
    if (!result.downloaded || !result.path) {
      throw new Error(localized('%@ could not be downloaded.', file.displayName()));
    }
    return result.path;
  }

  async _ensurePreviewOfFile(file: File, filePath: string) {
    if (!AppEnv.config.get('core.attachments.displayFilePreview')) {
      return;
    }
    if (!canPossiblyPreviewExtension(file)) {
      return;
    }

    const previewPath = `${filePath}.png`;

    if (await fileAccessibleAtPath(previewPath)) {
      // If the preview file already exists, set our state and bail
      this._filePreviewPaths[file.id] = previewPath;
      this.trigger();
      return;
    }

    // If the preview file doesn't exist yet, generate it
    if (await generatePreview({ file, filePath, previewPath })) {
      this._filePreviewPaths[file.id] = previewPath;
      this.trigger();
    }
  }

  // Section: Retrieval of Files

  _quickPreviewFile = (filePath: string) => {
    displayQuickPreviewWindow(filePath);
  };

  _fetch = (file: File) => {
    return (
      this._prepareAndResolveFilePath(file)
        .catch(this._catchFSErrors)
        // Passively ignore
        .catch(() => {})
    );
  };

  _fetchAndOpen = (file: File) => {
    return this._prepareAndResolveFilePath(file)
      .then(async (filePath) => {
        // The engine writes files/ itself, so the plist's LSFileQuarantineEnabled doesn't cover it.
        await markFileFromInternet(filePath, { keepExisting: true });
        if (await confirmOpeningFile(filePath)) {
          await shell.openPath(filePath);
        }
      })
      .catch(this._catchFSErrors)
      .catch((error) => {
        this._presentError({ file, error });
      });
  };

  // Copies an attachment out of files/ and marks the copy as downloaded (renderer writes
  // aren't covered by the main bundle's LSFileQuarantineEnabled).
  _writeToExternalPath = (filePath: string, savePath: string) => {
    return new Promise<void>((resolve, reject) => {
      const stream = fs.createReadStream(filePath);
      const out = fs.createWriteStream(savePath);
      stream.on('error', (err: Error) => reject(err));
      out.on('error', (err: Error) => reject(err));
      out.on('finish', () => resolve());
      stream.pipe(out);
    }).then(() => markFileFromInternet(savePath).then(() => {}));
  };

  _fetchAndSave = (file: File) => {
    const defaultPath = this._defaultSavePath(file);
    const defaultExtension = path.extname(defaultPath);

    AppEnv.showSaveDialog({ defaultPath }, (savePath) => {
      if (!savePath) {
        return;
      }

      const saveExtension = path.extname(savePath);
      const newDownloadDirectory = path.dirname(savePath);
      const didLoseExtension = defaultExtension !== '' && saveExtension === '';
      let actualSavePath = savePath;
      if (didLoseExtension) {
        actualSavePath += defaultExtension;
      }

      this._prepareAndResolveFilePath(file)
        .then((filePath) => this._writeToExternalPath(filePath, actualSavePath))
        .then(() => {
          if (AppEnv.savedState.lastDownloadDirectory !== newDownloadDirectory) {
            AppEnv.savedState.lastDownloadDirectory = newDownloadDirectory;

            if (
              this._lastDownloadDirectory !== newDownloadDirectory &&
              AppEnv.config.get('core.attachments.openFolderAfterDownload')
            ) {
              this._lastDownloadDirectory = newDownloadDirectory;
              shell.showItemInFolder(actualSavePath);
            }
          }
        })
        .catch(this._catchFSErrors)
        .catch((error) => {
          this._presentError({ file, error });
        });
    });
  };

  _fetchAndSaveAll = (files: File[]) => {
    const defaultPath = this._defaultSaveDir();

    return new Promise((resolve) => {
      AppEnv.showOpenDialog(
        {
          defaultPath,
          title: localized('Save Into...'),
          buttonLabel: localized('Download All'),
          properties: ['openDirectory', 'createDirectory'],
        },
        (selected) => {
          if (!selected) {
            return;
          }
          const dirPath = selected[0];
          if (!dirPath) {
            return;
          }
          this._lastDownloadDirectory = dirPath;
          AppEnv.savedState.lastDownloadDirectory = dirPath;

          const seenPaths = new Set();
          const lastSavePaths: string[] = [];
          const savePromises = files.map((file) => {
            let externalPath = path.join(dirPath, file.safeDisplayName());
            while (seenPaths.has(externalPath) || fs.existsSync(externalPath)) {
              externalPath = this._incrementPathToAvoidCollision(externalPath);
            }
            seenPaths.add(externalPath);

            return this._prepareAndResolveFilePath(file)
              .then((filePath) => this._writeToExternalPath(filePath, externalPath))
              .then(() => lastSavePaths.push(externalPath));
          });

          Promise.all(savePromises)
            .then(() => {
              if (
                lastSavePaths.length > 0 &&
                AppEnv.config.get('core.attachments.openFolderAfterDownload')
              ) {
                shell.showItemInFolder(lastSavePaths[0]);
              }
              return resolve(lastSavePaths);
            })
            .catch(this._catchFSErrors)
            .catch((error) => {
              this._presentError({ error });
            });
        }
      );
    });
  };

  _incrementPathToAvoidCollision(attemptedPath: string) {
    const ext = path.extname(attemptedPath);
    const dir = path.dirname(attemptedPath);
    let name = path.basename(attemptedPath, ext);
    // Use " (N)" format for collision counters to avoid conflicts with filenames
    // that end with hyphen-number patterns (like dates: "report-2023.pdf")
    const match = / \((\d+)\)$/.exec(name);
    let counter = 0;
    if (match) {
      counter = Number(match[1]);
      name = name.substr(0, match.index);
    }
    return path.join(dir, `${name} (${counter + 1})${ext}`);
  }

  _defaultSaveDir() {
    // Without the variable, Downloads is looked for under the working directory and the
    // temp dir is used, rather than path.join throwing.
    let home = '';
    if (process.platform === 'win32') {
      home = process.env.USERPROFILE || '';
    } else {
      home = process.env.HOME || '';
    }

    let downloadDir = path.join(home, 'Downloads');
    if (!fs.existsSync(downloadDir)) {
      downloadDir = os.tmpdir();
    }

    if (AppEnv.savedState.lastDownloadDirectory) {
      if (fs.existsSync(AppEnv.savedState.lastDownloadDirectory)) {
        downloadDir = AppEnv.savedState.lastDownloadDirectory;
      }
    }

    return downloadDir;
  }

  _defaultSavePath(file: File) {
    const downloadDir = this._defaultSaveDir();
    return path.join(downloadDir, file.safeDisplayName());
  }

  _presentError({ file, error }: { file?: File; error?: Error } = {}) {
    const name = file ? file.displayName() : localized('one or more files');
    const errorString = error ? error.toString() : '';

    Dialogs.showMessageBox(
      {
        type: 'warning',
        message: localized('Download Failed'),
        detail: localized(
          `Unable to download %@. Check your network connection and try again. %@`,
          name,
          errorString
        ),
        buttons: ['OK'],
      },
      'none'
    );
  }

  // Resolves once the user has seen a dialog for a file system error it explains; rejects with
  // any other error.
  async _catchFSErrors(error: NodeJS.ErrnoException) {
    let message: string | null = null;
    const code = error.code || '';
    if (['EPERM', 'EROFS', 'EPIPE', 'EBUSY', 'EMFILE', 'EACCES', 'UNKNOWN'].includes(code)) {
      message = localized(
        'Airpost could not save an attachment. Check that permissions are set correctly and try restarting Airpost if the issue persists.'
      );
    }
    if (['ENOSPC'].includes(code)) {
      message = localized(
        'Airpost could not save an attachment because you have run out of disk space.'
      );
    }

    if (message) {
      await Dialogs.showMessageBox(
        {
          type: 'warning',
          message: localized('Download Failed'),
          detail: `${message}\n\n${error.message}`,
          buttons: [localized('OK')],
        },
        'none'
      );
      return;
    }
    throw error;
  }

  // Section: Adding Files

  _assertIdPresent(headerMessageId: string) {
    if (!headerMessageId) {
      throw new Error('You need to pass the headerID of the message (draft) this Action refers to');
    }
  }

  _getFileStats(filepath: string) {
    return fs
      .statAsync(filepath)
      .catch(() =>
        Promise.reject(
          new Error(`${filepath} could not be found, or has invalid file permissions.`)
        )
      );
  }

  _copyToInternalPath(originPath: string, targetPath: string) {
    return new Promise<void>((resolve, reject) => {
      const readStream = fs.createReadStream(originPath);
      const writeStream = fs.createWriteStream(targetPath);

      readStream.on('error', () => reject(new Error(`Could not read file at path: ${originPath}`)));
      writeStream.on('error', () =>
        reject(new Error(`Could not write ${path.basename(targetPath)} to files directory.`))
      );
      readStream.on('end', () => resolve());
      readStream.pipe(writeStream);
    });
  }

  async _deleteFile(file: File) {
    // Delete the file, it's preview if present and it's containing folder. We don't
    // delete other arbitrary files in case for some reason other files are saved here.
    try {
      const filePath = this._requirePathForFile(file);
      await fs.unlinkAsync(filePath);
      if (await fileAccessibleAtPath(`${filePath}.png`)) {
        await fs.unlinkAsync(`${filePath}.png`);
      }
      await fs.rmdirAsync(path.dirname(filePath));
    } catch (err) {
      throw new Error(`Error deleting file file ${file.filename}:\n\n${(err as Error).message}`);
    }
  }

  async _applySessionChanges(headerMessageId: string, changeFunction: (files: File[]) => File[]) {
    const session = await DraftStore.sessionForClientId(headerMessageId);
    const draft = session.draft();
    if (!draft) {
      throw new Error(`AttachmentStore: draft ${headerMessageId} is not loaded`);
    }
    const files = changeFunction(draft.files);
    session.changes.add({ files });
  }

  // Handlers

  _onSelectAttachment = ({ headerMessageId }: { headerMessageId: string }) => {
    this._assertIdPresent(headerMessageId);

    // When the dialog closes, it triggers `Actions.addAttachment`
    return AppEnv.showOpenDialog({ properties: ['openFile', 'multiSelections'] }, (paths) => {
      if (paths == null) {
        return;
      }
      let pathsToOpen = paths;
      if (typeof pathsToOpen === 'string') {
        pathsToOpen = [pathsToOpen];
      }

      pathsToOpen.forEach((filePath) => Actions.addAttachment({ headerMessageId, filePath }));
    });
  };

  // Adds are serialized per draft so files land on the draft in dispatch
  // order rather than the order their copies happen to finish.
  _onAddAttachment = ({
    headerMessageId,
    filePath,
    inline = false,
    onCreated = (file: File) => {},
  }: {
    headerMessageId: string;
    filePath: string;
    inline?: boolean;
    onCreated?: (file: File) => void;
  }) => {
    this._assertIdPresent(headerMessageId);

    // Chain links swallow rejections so a single failed add can't poison the
    // rest of the chain for this draft.
    const previous = this._pendingAddsByDraft[headerMessageId] || Promise.resolve();
    const current = previous
      .then(() => this._addAttachmentNow({ headerMessageId, filePath, inline, onCreated }))
      .catch((err) => AppEnv.reportError(err));
    this._pendingAddsByDraft[headerMessageId] = current;
    current.then(() => {
      if (this._pendingAddsByDraft[headerMessageId] === current) {
        delete this._pendingAddsByDraft[headerMessageId];
      }
    });
    return current;
  };

  // Errors are surfaced in a dialog rather than rejected so one failed file
  // never blocks the rest of the chain for the same draft.
  async _addAttachmentNow({
    headerMessageId,
    filePath,
    inline,
    onCreated,
  }: {
    headerMessageId: string;
    filePath: string;
    inline: boolean;
    onCreated: (file: File) => void;
  }) {
    try {
      const filename = path.basename(filePath);
      const stats = await this._getFileStats(filePath);
      if (stats.isDirectory()) {
        throw new Error(
          localized(`%@ is a directory. Try compressing it and attaching it again.`, filename)
        );
      } else if (stats.size > 25 * 1000000) {
        throw new Error(
          localized(`%@ cannot be attached because it is larger than 25MB.`, filename)
        );
      }

      const file = new File({
        id: Utils.generateTempId(),
        filename: filename,
        size: stats.size,
        contentType: null,
        messageId: null,
        contentId: inline ? Utils.generateContentId() : null,
      });

      const internalPath = this._requirePathForFile(file);
      await _fs.promises.mkdir(path.dirname(internalPath), { recursive: true });
      await this._copyToInternalPath(filePath, internalPath);

      await this._applySessionChanges(headerMessageId, (files) => {
        if (files.reduce((c, f) => c + f.size, 0) >= 25 * 1000000) {
          throw new Error(localized(`Sorry, you can't attach more than 25MB of attachments`));
        }
        return files.concat([file]);
      });
      onCreated(file);
    } catch (err) {
      AppEnv.showErrorDialog((err as Error).message);
    }
  }

  _onRemoveAttachment = async (headerMessageId: string, fileToRemove: File) => {
    if (!fileToRemove) {
      return;
    }

    await this._applySessionChanges(headerMessageId, (files) =>
      files.filter(({ id }) => id !== fileToRemove.id)
    );

    try {
      await this._deleteFile(fileToRemove);
    } catch (err) {
      AppEnv.showErrorDialog((err as Error).message);
    }
  };
}

export default new AttachmentStore();
