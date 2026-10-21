import * as React from 'react';
import * as cheerio from 'cheerio';
// cheerio's own node types; it doesn't re-export them.
import type { ParentNode } from 'domhandler';
import { LRUCache } from 'lru-cache';
import { Message } from '../../data/models/message';
import ComponentRegistry from '../../packages/component-registry';
import { UnsubscribeHeader } from './header';
import {
  UnsubscribeOption,
  parseListUnsubscribeHeader,
  supportsOneClick,
  getBestUnsubscribeOption,
} from './service';

const regexps = [
  // English
  /unsubscribe/gi,
  /unfollow/gi,
  /opt[ -]{0,2}out/gi,
  /email preferences/gi,
  /subscription/gi,
  /notification settings/gi,
  /Remove yourself from this mailing/gi,
  // Danish
  /afmeld/gi,
  /afmelden/gi,
  /af te melden voor/gi,
  // Spanish
  /darse de baja/gi,
  // French
  /désabonnement/gi,
  /désinscrire/gi,
  /désinscription/gi,
  /désabonner/gi,
  /préférences d'email/gi,
  /préférences d'abonnement/gi,
  // Russian - this is probably wrong:
  /отказаться от подписки/gi,
  // Serbian
  /одјавити/gi,
  // Icelandic
  /afskrá/gi,
  // Hebrew
  /לבטל את המנוי/gi,
  // Creole (Haitian)
  /koupe abònman/gi,
  // Chinese (Simplified)
  /退订/gi,
  // Chinese (Traditional)
  /退訂/gi,
  // Arabic
  /إلغاء الاشتراك/gi,
  // Armenian
  /պետք է նախ միանալ/gi,
  // German
  /abbestellen/gi,
  /abbestellung/gi,
  /abmelden/gi,
  /abmeldung/gi,
  /ausschreiben/gi,
  /austragen/gi,
  // Swedish
  /avprenumerera/gi,
  /avregistrera/gi,
  /prenumeration/gi,
  /notisinställningar/gi,
];

/** Cache for body-parsed unsubscribe links (limited to prevent memory growth) */
const bodyLinkCache = new LRUCache<string, { link: string | null }>({ max: 150 });

interface UnsubscribeAction {
  /** Undefined for an <a> without an href. */
  href: string | undefined;
  innerText: string;
}

/**
 * Parses the email body to find unsubscribe links.
 * Used as fallback when List-Unsubscribe header is not available.
 */
function findBodyUnsubscribeLink(message: Message): string | null {
  const cached = bodyLinkCache.get(message.id);
  if (cached) {
    return cached.link;
  }

  let result: string | null = null;

  // Only check the body if it has been downloaded already
  if (message.body) {
    const dom = cheerio.load(message.body);
    const links = _getLinks(dom);

    for (const link of links) {
      for (const re of regexps) {
        if (re.test(String(link.href)) && link.href !== undefined) {
          // If the URL contains e.g. "unsubscribe" we assume that we have correctly
          // detected the unsubscribe link.
          bodyLinkCache.set(message.id, { link: link.href });
          return link.href;
        }
        if (re.test(link.innerText)) {
          // If the URL does not contain "unsubscribe", but the text around the link contains
          // it, it is a possible candidate, we still check all other links for a better match
          result = link.href ?? null;
        }
      }
    }

    bodyLinkCache.set(message.id, { link: result });
  }

  return result;
}

type UnsubscribeMethod = 'one-click' | 'mailto' | 'web' | 'body-link';

interface UnsubscribeInfo {
  option: UnsubscribeOption;
  method: UnsubscribeMethod;
}

/**
 * Determines the best unsubscribe method for a message.
 *
 * Priority order:
 * 1. Header-based one-click (RFC 8058) - HTTPS with List-Unsubscribe-Post
 * 2. Header-based mailto - opens composer
 * 3. Header-based web - opens browser
 * 4. Body-parsed link - fallback, opens browser
 */
function getUnsubscribeInfo(message: Message): UnsubscribeInfo | null {
  // 1. Try header-based unsubscribe (RFC 2369 / RFC 8058)
  if (message.listUnsubscribe) {
    const headerOptions = parseListUnsubscribeHeader(message.listUnsubscribe);
    const hasOneClickSupport = supportsOneClick(message.listUnsubscribePost);

    const best = getBestUnsubscribeOption(headerOptions, hasOneClickSupport);
    if (best) {
      return {
        option: best.option,
        method: best.method,
      };
    }
  }

  // 2. Fall back to body-parsed link
  const bodyLink = findBodyUnsubscribeLink(message);
  if (bodyLink) {
    return {
      option: { type: 'https', uri: bodyLink },
      method: 'body-link',
    };
  }

  return null;
}

/**
 * Container component that determines the best unsubscribe method
 * and renders the UnsubscribeHeader component.
 */
const UnsubscribeHeaderContainer: React.FunctionComponent<{ message: Message }> = ({ message }) => {
  const info = getUnsubscribeInfo(message);

  if (!info) {
    return null;
  }

  return (
    <UnsubscribeHeader
      option={info.option}
      method={info.method}
      message={{ accountId: message.accountId, id: message.id }}
    />
  );
};

UnsubscribeHeaderContainer.displayName = 'UnsubscribeContainer';

export function activate() {
  ComponentRegistry.register(UnsubscribeHeaderContainer, { role: 'message:BodyHeader' });
}

export function deactivate() {
  ComponentRegistry.unregister(UnsubscribeHeaderContainer);
}

// Takes a parsed DOM (through cheerio) and returns links paired with contextual text
// Good at catching cases such as:
//    "If you would like to unsubscrbe from our emails, please click here."
// Returns a list of links as {href, innerText} objects
function _getLinks($: cheerio.CheerioAPI): UnsubscribeAction[] {
  const aParents: ParentNode[] = [];
  $('a:not(blockquote a)').each((_index, aTag) => {
    if (aTag && aTag.parent && !$(aParents).is(aTag.parent)) {
      aParents.unshift(aTag.parent);
    }
  });

  const links: UnsubscribeAction[] = [];
  $(aParents).each((_parentIndex, parent) => {
    let link: string | false | undefined = false;
    let leftoverText = '';
    $(parent.children).each((_childIndex, child) => {
      if ($(child).is($('a'))) {
        if (link !== false && leftoverText.length > 0) {
          links.push({
            href: link,
            innerText: leftoverText,
          });
          leftoverText = '';
        }
        link = $(child).attr('href');
      }
      const text = $(child).text();
      const re = /(.*[.!?](?:\s|$))/g;
      if (re.test(text)) {
        const splitup = text.split(re);
        for (let i = 0; i < splitup.length; i += 1) {
          if (splitup[i] !== '' && splitup[i] !== undefined) {
            if (link !== false) {
              const fullLine = leftoverText + splitup[i];
              links.push({
                href: link,
                innerText: fullLine,
              });
              link = false;
              leftoverText = '';
            } else {
              leftoverText += splitup[i];
            }
          }
        }
      } else {
        leftoverText += text;
      }
      leftoverText += ' ';
    });
    if (link !== false && leftoverText.length > 0) {
      links.push({
        href: link,
        innerText: leftoverText,
      });
    }
  });
  return links;
}
