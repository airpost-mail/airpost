import {
  Contact,
  ExtensionRegistry,
  Message,
  MessageBodyProcessor,
  MessageStore,
} from 'mailspring-exports';
import AutoloadImagesExtension from '../lib/autoload-images-extension';
import AutoloadImagesStore from '../lib/autoload-images-store';

const IMAGE_BODY = '<p>Hi</p><img src="https://tracker.example.com/open.gif">';
// Forms the regex misses; the CSP blocks them anyway, but no banner shows for them.
const CSS_ONLY_BODY = '<div style="background:#fff url(https://tracker.example.com/p.png)"></div>';

describe('AutoloadImages', function () {
  let autoload: boolean;
  let message: Message;

  beforeEach(() => {
    autoload = false;
    const get = AppEnv.config.get.bind(AppEnv.config);
    spyOn(AppEnv.config, 'get').andCallFake((key: string) =>
      key === 'core.reading.autoloadImages' ? autoload : get(key)
    );
    spyOn(MessageBodyProcessor, 'resetCache');
    AutoloadImagesStore._whitelistEmails = {};
    AutoloadImagesStore._whitelistMessageIds = {};
    message = new Message({
      id: 'autoload-images-spec',
      accountId: 'acct',
      body: IMAGE_BODY,
      from: [new Contact({ email: 'news@shop.example.com', name: 'Shop' })],
    } as any);
    spyOn(message, 'folderIds').andReturn([]);
  });

  it('blocks remote images by default', () => {
    expect(AutoloadImagesExtension.allowsRemoteImages({ message })).toBe(false);
    expect(AutoloadImagesStore.shouldBlockImagesIn(message)).toBe(true);
  });

  it('allows them when the setting is on', () => {
    autoload = true;
    expect(AutoloadImagesExtension.allowsRemoteImages({ message })).toBe(true);
    expect(AutoloadImagesStore.shouldBlockImagesIn(message)).toBe(false);
  });

  it('allows them for a message after "Show Images" and re-renders', () => {
    AutoloadImagesStore._onTemporarilyEnableImages(message);
    expect(AutoloadImagesExtension.allowsRemoteImages({ message })).toBe(true);
    expect(MessageBodyProcessor.resetCache).toHaveBeenCalled();
  });

  it('allows them for every message from an allowed sender', () => {
    spyOn(AutoloadImagesStore, '_saveWhitelist');
    AutoloadImagesStore._onPermanentlyEnableImages(message);
    const other = new Message({
      id: 'autoload-images-spec-2',
      accountId: 'acct',
      body: IMAGE_BODY,
      from: [new Contact({ email: 'NEWS@shop.example.com', name: 'Shop' })],
    } as any);
    spyOn(other, 'folderIds').andReturn([]);
    expect(AutoloadImagesExtension.allowsRemoteImages({ message: other })).toBe(true);
    expect(MessageBodyProcessor.resetCache).toHaveBeenCalled();
  });

  it('no longer rewrites the body; the banner only shows when the regex finds images', () => {
    expect((AutoloadImagesExtension as any).formatMessageBody({ message })).toBe(undefined);
    expect(message.body).toEqual(IMAGE_BODY);
    message.body = CSS_ONLY_BODY;
    expect(AutoloadImagesStore.shouldBlockImagesIn(message)).toBe(false);
    expect(AutoloadImagesExtension.allowsRemoteImages({ message })).toBe(false);
  });

  it('decides the policy MessageBodyProcessor hands the message frame', async () => {
    const registered = ExtensionRegistry.MessageView.extensions().includes(AutoloadImagesExtension);
    if (!registered) ExtensionRegistry.MessageView.register(AutoloadImagesExtension);
    try {
      expect((await MessageBodyProcessor.processUnclipped(message)).remoteImages).toBe(false);
      AutoloadImagesStore._whitelistMessageIds[message.id] = true;
      expect((await MessageBodyProcessor.processUnclipped(message)).remoteImages).toBe(true);
    } finally {
      if (!registered) ExtensionRegistry.MessageView.unregister(AutoloadImagesExtension);
    }
  });

  it('falls back to the setting when no extension answers', () => {
    spyOn(MessageStore, 'extensions').andReturn([]);
    expect(MessageBodyProcessor.remoteImagesAllowed(message)).toBe(false);
    autoload = true;
    expect(MessageBodyProcessor.remoteImagesAllowed(message)).toBe(true);
  });
});
