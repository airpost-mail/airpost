import { Message } from 'mailspring-exports';
import LinkTrackingMessageExtension from '../lib/message-extension';
import { PLUGIN_ID } from '../lib/constants';

// The extension works on the message frame through MessageBody calls.
describe('LinkTrackingMessageExtension', function () {
  it('restores tracked hrefs and adds a dot after each tracked link', async () => {
    const message = new Message({ id: 'lt-1', body: '' });
    spyOn(message, 'metadataForPluginId').andCallFake((id: string) =>
      id === PLUGIN_ID
        ? {
            links: [
              { url: 'https://example.com/a', redirect_url: 'https://t.example/1', click_count: 2 },
            ],
          }
        : null
    );
    const calls: unknown[][] = [];
    const body = {
      querySelectorAll: async () => [
        { ref: 1, tagName: 'A', attributes: { href: 'https://t.example/1' }, text: 'a' },
        { ref: 2, tagName: 'A', attributes: { href: 'https://other.example' }, text: 'b' },
      ],
      setAttributes: async (ref: number, attrs: { [name: string]: string | null }) =>
        calls.push(['set', ref, attrs]),
      insertElement: async (
        ref: number,
        position: InsertPosition,
        tag: string,
        attrs: { [name: string]: string }
      ) => {
        calls.push(['insert', ref, position, tag, attrs.title]);
        return 10;
      },
      addEventListener: async (ref: number, event: string) => calls.push(['listen', ref, event]),
    };
    await LinkTrackingMessageExtension.renderedMessageBodyIntoDocument({
      body: body as any,
      message,
    });
    expect(calls).toEqual([
      ['set', 1, { href: 'https://example.com/a', title: 'https://example.com/a' }],
      ['insert', 1, 'afterend', 'img', '2 clicks (https://example.com/a)'],
      ['listen', 10, 'mousedown'],
    ]);
  });
});
