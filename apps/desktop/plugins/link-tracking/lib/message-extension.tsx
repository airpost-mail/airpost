import React from 'react';
import { MessageViewExtension, Actions, Message } from 'mailspring-exports';
import type { MessageBody } from '../../../src/mail/frame/bridge';
import LinkTrackingMessagePopover from './message-popover';
import { PLUGIN_ID } from './constants';
import { LinkTrackingLink, LinkTrackingMetadata } from './types';

const DotStyle =
  'margin-bottom: 0.75em; margin-left: 1px; margin-right: 1px; vertical-align: text-bottom; width: 6px;';

export default class LinkTrackingMessageExtension extends MessageViewExtension {
  static async renderedMessageBodyIntoDocument({
    body,
    message,
  }: {
    body: MessageBody;
    message: Message;
  }) {
    const metadata: LinkTrackingMetadata | null = message.metadataForPluginId(PLUGIN_ID);
    const trackedLinks = metadata && metadata.links;
    if (!trackedLinks || trackedLinks.length === 0) {
      return;
    }

    const links: { [href: string]: LinkTrackingLink } = {};
    for (const link of trackedLinks) {
      links[link.url] = link;
      links[link.redirect_url] = link;
    }

    for (const anchor of await body.querySelectorAll('a[href]')) {
      const link = links[anchor.attributes.href];
      if (!link) continue;
      const originalHref = link.url;
      const clicked = link.click_count > 0;

      await body.setAttributes(anchor.ref, { href: originalHref, title: originalHref });
      const dot = await body.insertElement(anchor.ref, 'afterend', 'img', {
        class: 'link-tracking-dot',
        style: `${DotStyle}${clicked ? ' cursor: pointer;' : ''}`,
        title: clicked
          ? `${link.click_count} click${link.click_count === 1 ? '' : 's'} (${originalHref})`
          : `This link has not been clicked (${originalHref})`,
        src: clicked
          ? 'tern-asset://link-tracking/assets/ic-tracking-visited@2x.png'
          : 'tern-asset://link-tracking/assets/ic-tracking-unvisited@2x.png',
      });
      if (clicked) {
        await body.addEventListener(dot, 'mousedown', ({ rect }) => {
          Actions.openPopover(
            <LinkTrackingMessagePopover message={message} linkMetadata={link} />,
            { originRect: rect, direction: 'down' }
          );
        });
      }
    }
  }
}
