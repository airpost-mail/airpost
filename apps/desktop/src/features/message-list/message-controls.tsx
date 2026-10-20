/* eslint global-require: 0 */

import React from 'react';
import ReactDOM from 'react-dom';
import {
  localized,
  Actions,
  TaskQueue,
  GetMessageRFC2822Task,
  EmlUtils,
  Thread,
  Message,
  ComponentRegistry,
} from 'mailspring-exports';
import { RetinaImg, ButtonDropdown, Menu } from 'mailspring-component-kit';
import { escapeHTML } from 'underscore.string';
import { developerToolsAvailable } from '../../shared/developer-tools';
import {
  contentSecurityPolicyMetaTag,
  messageContentSecurityPolicy,
} from '../../shared/content-security-policy';
import { OriginalColors, darkeningApplies, themeBackgroundAt } from './email-render-mode';

interface MessageControlsProps {
  thread: Thread;
  message: Message;
}

export default class MessageControls extends React.Component<MessageControlsProps> {
  static displayName = 'MessageControls';

  _items() {
    const reply = {
      name: localized('Reply'),
      image: 'ic-dropdown-reply.png',
      select: this._onReply,
    };
    const replyAll = {
      name: localized('Reply All'),
      image: 'ic-dropdown-replyall.png',
      select: this._onReplyAll,
    };
    const forward = {
      name: localized('Forward'),
      image: 'ic-dropdown-forward.png',
      select: this._onForward,
    };

    const showOriginal = {
      name: localized('Show Original'),
      image: 'ic-dropdown-whitespace.png',
      select: this._onShowOriginal,
    };

    if (!this.props.message.canReplyAll()) {
      return [reply, forward, showOriginal];
    }
    const defaultReplyType = AppEnv.config.get('core.sending.defaultReplyType');
    return defaultReplyType === 'reply-all'
      ? [replyAll, reply, forward, showOriginal]
      : [reply, replyAll, forward, showOriginal];
  }

  _dropdownMenu(items: Array<{ name: string; image: string; select: () => void }>) {
    const itemContent = (item: { name: string; image: string; select: () => void }) => (
      <span>
        <RetinaImg name={item.image} mode={RetinaImg.Mode.ContentIsMask} />
        &nbsp;&nbsp;{item.name}&nbsp;&nbsp;
      </span>
    );

    return (
      <Menu
        items={items}
        itemKey={(item) => item.name}
        itemContent={itemContent}
        onSelect={(item) => item.select()}
      />
    );
  }

  _onReply = () => {
    const { thread, message } = this.props;
    Actions.composeReply({
      thread,
      message,
      type: 'reply',
      behavior: 'prefer-existing-if-pristine',
    });
  };

  _onReplyAll = () => {
    const { thread, message } = this.props;
    Actions.composeReply({
      thread,
      message,
      type: 'reply-all',
      behavior: 'prefer-existing-if-pristine',
    });
  };

  _onForward = () => {
    const { thread, message } = this.props;
    Actions.composeForward({ thread, message });
  };

  _onDownloadEml = () => {
    const { message } = this.props;
    const defaultFilename = EmlUtils.defaultEmlFilename(message.subject);

    AppEnv.showSaveDialog(
      { defaultPath: defaultFilename, title: localized('Save Email') },
      async (savePath: string) => {
        if (!savePath) return;
        const task = new GetMessageRFC2822Task({
          messageId: message.id,
          accountId: message.accountId,
          filepath: savePath,
        });
        Actions.queueTask(task);
        await TaskQueue.waitForPerformRemote(task);
      }
    );
  };

  _onShowActionsMenu = () => {
    const SystemMenu = require('@electron/remote').Menu;
    const SystemMenuItem = require('@electron/remote').MenuItem;

    // Todo: refactor this so that message actions are provided
    // dynamically. Waiting to see if this will be used often.
    const menu = new SystemMenu();
    menu.append(new SystemMenuItem({ label: localized('Log Data'), click: this._onLogData }));
    menu.append(
      new SystemMenuItem({ label: localized('Show Original'), click: this._onShowOriginal })
    );
    const colorsItem = this._originalColorsMenuItem();
    if (colorsItem) menu.append(new SystemMenuItem(colorsItem));
    menu.append(
      new SystemMenuItem({
        label: localized('Copy Debug Info to Clipboard'),
        click: this._onCopyToClipboard,
      })
    );
    menu.append(new SystemMenuItem({ type: 'separator' }));
    menu.append(
      new SystemMenuItem({ label: localized('Download as .eml'), click: this._onDownloadEml })
    );

    const actionItems = ComponentRegistry.findComponentsMatching({
      role: 'MessageActionMenuItem',
    }) as Array<{
      getMenuItem: (props: MessageControlsProps) => { label: string; click: () => void };
    }>;
    if (actionItems.length > 0) {
      menu.append(new SystemMenuItem({ type: 'separator' }));
      for (const ActionItem of actionItems) {
        const item = ActionItem.getMenuItem(this.props);
        menu.append(new SystemMenuItem({ label: item.label, click: item.click }));
      }
    }

    menu.popup({});
  };

  // Offered only when the theme and setting darken emails.
  _originalColorsMenuItem() {
    const el = ReactDOM.findDOMNode(this) as HTMLElement | null;
    if (!el || !darkeningApplies(themeBackgroundAt(el))) return null;
    const { message } = this.props;
    const original = OriginalColors.isShowingOriginal(message.id);
    return {
      label: original ? localized('Show Dark Colors') : localized('Show Original Colors'),
      click: () => OriginalColors.setShowingOriginal([message.id], !original),
    };
  }

  _onShowOriginal = async () => {
    const { message } = this.props;
    const filepath = require('path').join(
      require('@electron/remote').app.getPath('temp'),
      message.id
    );
    const task = new GetMessageRFC2822Task({
      messageId: message.id,
      accountId: message.accountId,
      filepath,
    });
    Actions.queueTask(task);
    await TaskQueue.waitForPerformRemote(task);

    // The task can reach "complete" status even when it failed remotely (eg.
    // the message could not be fetched from the server), in which case the
    // file is never written. Verify it exists before trying to display it.
    const fs = require('fs');
    if (!fs.existsSync(filepath)) {
      AppEnv.showErrorDialog(
        localized('Could not retrieve the original message. Please try again.')
      );
      return;
    }

    // Shown as escaped text in a page carrying the message CSP, rather than loading the raw
    // file and leaving Chromium to guess whether it is HTML.
    // Not `${id}.html`: the clipped-message window (message-item-body.tsx) writes that.
    const htmlPath = `${filepath}.rfc822.html`;
    const csp = contentSecurityPolicyMetaTag(messageContentSecurityPolicy({ remoteImages: false }));
    try {
      const raw = await fs.promises.readFile(filepath, 'utf8');
      await fs.promises.writeFile(
        htmlPath,
        `<!DOCTYPE html>${csp}<meta charset="UTF-8">` +
          `<pre style="white-space: pre-wrap; word-wrap: break-word; margin: 8px;">` +
          `${escapeHTML(raw)}</pre>`
      );
    } catch {
      AppEnv.showErrorDialog(
        localized('Could not retrieve the original message. Please try again.')
      );
      return;
    }

    const { BrowserWindow } = require('@electron/remote');
    const win = new BrowserWindow({
      width: 800,
      height: 600,
      title: `${message.subject} - RFC822`,
      webPreferences: {
        javascript: false,
        nodeIntegration: false,
        devTools: developerToolsAvailable(),
      },
    });
    win.loadURL(`file://${htmlPath}`).catch((err: Error) => {
      console.error('Show Original window failed to load:', err);
    });
  };

  _onLogData = () => {
    console.log(this.props.message);
    (window as any).__message = this.props.message;
    (window as any).__thread = this.props.thread;
    console.log('Also now available in window.__message and window.__thread');
  };

  _onCopyToClipboard = () => {
    const { message, thread } = this.props;
    const data = `
      AccountID: ${message.accountId}
      Message ID: ${message.id}
      Message Metadata: ${JSON.stringify(message.pluginMetadata, null, '  ')}
      Thread ID: ${thread.id}
      Thread Metadata: ${JSON.stringify(thread.pluginMetadata, null, '  ')}
    `;
    require('@electron/remote').clipboard.writeText(data);
  };

  render() {
    // A draft that is sending renders as a message; replying to or forwarding it would quote
    // a message that hasn't been sent, so it only gets the actions menu.
    const items = this.props.message.draft ? null : this._items();
    return (
      <div className="message-actions-wrap" onClick={(e) => e.stopPropagation()}>
        {items && (
          <ButtonDropdown
            primaryItem={<RetinaImg name={items[0].image} mode={RetinaImg.Mode.ContentIsMask} />}
            primaryTitle={items[0].name}
            primaryClick={items[0].select}
            closeOnMenuClick
            menu={this._dropdownMenu(items.slice(1))}
          />
        )}
        <div
          className="message-actions-ellipsis"
          tabIndex={-1}
          onClick={this._onShowActionsMenu}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              this._onShowActionsMenu();
            }
          }}
        >
          <RetinaImg
            name={'message-actions-ellipsis.png'}
            mode={RetinaImg.Mode.ContentIsMask}
            aria-hidden="true"
          />
        </div>
      </div>
    );
  }
}
