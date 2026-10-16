// The account the sidebar shows when the URL has none (settings, search).
const KEY = 'tern.mailbox.account';

export function getLastAccount(): string | null {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

export function setLastAccount(id: string) {
  try {
    localStorage.setItem(KEY, id);
  } catch {
    // Not persisted; the first account is a fine fallback.
  }
}
