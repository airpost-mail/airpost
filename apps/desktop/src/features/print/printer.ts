import { AccountStore } from '../../data/stores/account';
import * as Actions from '../../data/actions';
import { Thread } from '../../data/models/thread';
import { messageFrameBridgeFor, messageFrameContent } from '../../mail/frame/bridge';
import PrintWindow from './window';

class Printer {
  public unsub = Actions.printThread.listen(this._printThread);

  async _printThread(thread: Thread, htmlContent: string) {
    if (!thread) throw new Error('Printing: No thread active!');
    const account = AccountStore.accountForId(thread.accountId);

    // The <managed-styles> tag that holds the window's stylesheets
    const styleTag = document.getElementsByTagName('managed-styles')[0];
    // The message frames of the focused thread (email-frame.tsx).
    const iframes = Array.from(document.getElementsByTagName('iframe')).filter((iframe) =>
      messageFrameBridgeFor(iframe)
    );
    // Remote images print only if every message shown allows them.
    const remoteImages =
      iframes.length > 0 &&
      iframes.every((iframe) => iframe.getAttribute('data-remote-images') === 'allowed');
    // The frames are cross-origin, so each one sends its HTML, without the dark rendering of
    // the email colours: paper gets the sender's colours.
    const contents = await Promise.all(iframes.map((iframe) => messageFrameContent(iframe)));
    const messagesHtml = contents.filter(Boolean).map((content) => content.html);

    const win = new PrintWindow({
      subject: thread.subject,
      account: {
        name: account.name,
        email: account.emailAddress,
      },
      participants: thread.participants,
      styleTags: styleTag.innerHTML,
      htmlContent,
      printMessages: JSON.stringify(messagesHtml),
      remoteImages,
    });
    win.load();
  }

  deactivate() {
    this.unsub();
  }
}

export default Printer;
