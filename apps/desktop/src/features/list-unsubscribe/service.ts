import EngineEvents from '../../engine/client/events';
import {
  Method,
  MessagesUnsubscribeParams,
  MessagesUnsubscribeResult,
} from '../../engine/protocol';

const { shell } = require('electron');

/**
 * Represents a parsed unsubscribe option from the List-Unsubscribe header.
 */
export interface UnsubscribeOption {
  type: 'https' | 'http' | 'mailto';
  uri: string;
}

/**
 * Result of an unsubscribe attempt.
 */
export interface UnsubscribeResult {
  success: boolean;
  error?: string;
}

/**
 * Parses the List-Unsubscribe header value according to RFC 2369.
 *
 * The header format is: <uri1>, <uri2>, ...
 * Where each URI is enclosed in angle brackets and separated by commas.
 *
 * Example: "<mailto:unsubscribe@example.com>, <https://example.com/unsub>"
 *
 * @param header - The raw List-Unsubscribe header value
 * @returns An array of parsed unsubscribe options
 */
export function parseListUnsubscribeHeader(header: string): UnsubscribeOption[] {
  if (!header || typeof header !== 'string') {
    return [];
  }

  const options: UnsubscribeOption[] = [];

  // Match URIs enclosed in angle brackets: <...>
  const uriRegex = /<([^>]+)>/g;
  let match: RegExpExecArray | null;

  while ((match = uriRegex.exec(header)) !== null) {
    const uri = match[1].trim();

    if (uri.toLowerCase().startsWith('https://')) {
      options.push({ type: 'https', uri });
    } else if (uri.toLowerCase().startsWith('http://')) {
      options.push({ type: 'http', uri });
    } else if (uri.toLowerCase().startsWith('mailto:')) {
      options.push({ type: 'mailto', uri });
    }
  }

  return options;
}

/**
 * Checks if the List-Unsubscribe-Post header indicates one-click support (RFC 8058).
 *
 * @param listUnsubscribePost - The List-Unsubscribe-Post header value
 * @returns True if one-click unsubscribe is supported
 */
export function supportsOneClick(listUnsubscribePost: string | undefined): boolean {
  if (!listUnsubscribePost) {
    return false;
  }
  // RFC 8058 requires the exact value "List-Unsubscribe=One-Click"
  return listUnsubscribePost.trim() === 'List-Unsubscribe=One-Click';
}

/**
 * Opens the unsubscribe URL in the user's default browser.
 *
 * For a link found in the body, and for the web link `messages.unsubscribe` hands back.
 *
 * @param url - The HTTP/HTTPS URL to open
 */
export function performWebUnsubscribe(url: string): void {
  if (!/^https?:\/\/.+/i.test(url)) {
    throw new Error('Invalid URL');
  }
  shell.openExternal(url);
}

/**
 * Determines the best unsubscribe method based on available options.
 *
 * Priority order:
 * 1. One-click HTTPS (if List-Unsubscribe-Post header present) - instant, no user interaction
 * 2. Mailto - opens composer for user review
 * 3. Regular HTTPS/HTTP - opens in browser
 *
 * @param options - Parsed unsubscribe options from the header
 * @param hasOneClickSupport - Whether List-Unsubscribe-Post header indicates one-click support
 * @returns The best unsubscribe option, or null if none available
 */
export function getBestUnsubscribeOption(
  options: UnsubscribeOption[],
  hasOneClickSupport: boolean
): { option: UnsubscribeOption; method: 'one-click' | 'mailto' | 'web' } | null {
  if (options.length === 0) {
    return null;
  }

  // If one-click is supported, prefer HTTPS URL
  if (hasOneClickSupport) {
    const httpsOption = options.find((o) => o.type === 'https');
    if (httpsOption) {
      return { option: httpsOption, method: 'one-click' };
    }
  }

  // Otherwise, prefer mailto (opens composer for user review)
  const mailtoOption = options.find((o) => o.type === 'mailto');
  if (mailtoOption) {
    return { option: mailtoOption, method: 'mailto' };
  }

  // Fall back to web (opens in browser)
  const webOption = options.find((o) => o.type === 'https' || o.type === 'http');
  if (webOption) {
    return { option: webOption, method: 'web' };
  }

  return null;
}

/** The part of EngineEvents the engine path uses, so specs can pass a fake. */
export interface UnsubscribeEngine {
  request<R>(method: string, params: unknown): Promise<R>;
}

/**
 * Unsubscribes by the message's List-Unsubscribe header through the engine: it makes the
 * one-click POST or sends the mailto message itself (without the composer), and hands a web
 * link back, which opens in the browser here. Links found only in the body stay with
 * `performWebUnsubscribe`.
 */
export async function performEngineUnsubscribe(
  message: { accountId: string; id: string },
  engine: UnsubscribeEngine = EngineEvents,
  openWeb: (url: string) => void = performWebUnsubscribe
): Promise<UnsubscribeResult & { method?: string }> {
  const params: MessagesUnsubscribeParams = { accountId: message.accountId, messageId: message.id };
  try {
    const r = await engine.request<MessagesUnsubscribeResult>(Method.MessagesUnsubscribe, params);
    if (r.ok && r.method === 'web' && r.url) openWeb(r.url);
    return { success: r.ok, error: r.ok ? undefined : r.error, method: r.method };
  } catch (err) {
    return {
      success: false,
      // RpcError, EngineUnavailableError, or anything with a message.
      error: (err as { message?: string })?.message || String(err),
    };
  }
}
