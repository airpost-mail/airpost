import { Message } from '../data/models/message';
import { File } from '../data/models/file';
import type { MessageBody } from '../mail/frame/bridge';

/*
Public: To create MessageViewExtension that customize message viewing, you
should create objects that implement the interface defined at {MessageViewExtension}.

To register your extension with the ExtensionRegistry, call {ExtensionRegistry::MessageView::registerExtension}.
When your package is being unloaded, you *must* call the corresponding
{ExtensionRegistry::MessageView::unregisterExtension} to unhook your extension.

```javascript
activate() {
  ExtensionRegistry.MessageView.register(MyExtension)
}
...

deactivate() {
  ExtensionRegistry.MessageView.unregister(MyExtension)
}
```

The MessageViewExtension API does not currently expose any asynchronous or {Promise}-based APIs.
This will likely change in the future. If you have a use-case for a Message Store extension that
is not possible with the current API, please let us know.

Section: Extensions
*/
export class MessageViewExtension {
  /*
  Public: Modify the body of the message provided. Note that you're provided
  the entire message object, but you can only change `message.body`.
  */
  static formatMessageBody({ message }: { message: Message }) {}

  /*
  Public: Whether the message may load remote images and fonts. Return true to allow,
  false to block, or undefined to leave it to other extensions. The message document's
  Content-Security-Policy enforces the answer, so rewriting the body isn't needed to block.
  */
  static allowsRemoteImages({ message }: { message: Message }): boolean | undefined {
    return undefined;
  }

  /*
  Public: Modify the rendered message body. Runs after the message goes through
  `formatMessageBody` and is rendered, again on every re-render.

  Since plugin API 1.27 the body renders in a cross-origin, sandboxed frame in its own
  process, so there is no `document`. `body` (a {MessageBody}) finds elements, sets
  attributes, inserts elements and listens for clicks inside it; its calls return promises
  and reject once the message renders again. `iframe` is the frame element in the app window.
  Return a promise to have its rejection reported.
  */
  static renderedMessageBodyIntoDocument({
    body,
    message,
    iframe,
  }: {
    body: MessageBody;
    message: Message;
    iframe: HTMLIFrameElement;
  }): void | Promise<void> {}

  /*
  Public: Filter the list of displayed attachments.
  */
  static filterMessageFiles({ message, files }: { message: Message; files: File[] }): File[] {
    return files;
  }
}
