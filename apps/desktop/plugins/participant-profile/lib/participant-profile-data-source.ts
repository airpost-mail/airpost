import { Utils, CapabilityStore, Integrations } from 'mailspring-exports';

const CACHE_SIZE = 200;
const CACHE_INDEX_KEY = 'pp-cache-v3-keys';
const CACHE_KEY_PREFIX = 'pp-cache-v3-';

class ParticipantProfileDataSource {
  _cacheIndex: string[];

  constructor() {
    try {
      this._cacheIndex = JSON.parse(window.localStorage.getItem(CACHE_INDEX_KEY) || `[]`);
    } catch (err) {
      this._cacheIndex = [];
    }
  }

  async find(contact: { email: string; name: string }) {
    const { email } = contact;

    if (!email || Utils.likelyNonHumanEmail(email)) {
      return {};
    }

    const cached = this.getCache(email);
    if (cached) {
      return cached;
    }

    // Without People Data Labs set up the sidebar shows the local contact and message history.
    if (!CapabilityStore.isAvailable('contact-enrichment')) {
      return {};
    }

    // The engine keeps an in-memory cache too; this one survives restarts.
    try {
      const { person, company } = await Integrations.request('integrations.enrich', { email });
      const profile: { person?: unknown; company?: unknown } = {};
      if (person) profile.person = person;
      if (company) profile.company = company;
      // A no-match isn't kept: providers fill in over time, and the engine caches it for a day.
      if (person || company) this.setCache(email, profile);
      return profile;
    } catch (err) {
      // Not cached, so the next view retries. Only the key: the message may name the address.
      console.warn(`Contact lookup failed: ${Integrations.toIntegrationError(err).key}`);
      return {};
    }
  }

  // LocalStorage Retrieval / Saving

  hasCache(email: string) {
    return localStorage.getItem(`${CACHE_KEY_PREFIX}${email}`) !== null;
  }

  getCache(email: string) {
    const raw = localStorage.getItem(`${CACHE_KEY_PREFIX}${email}`);
    if (!raw) {
      return null;
    }
    try {
      return JSON.parse(raw);
    } catch (err) {
      return null;
    }
  }

  setCache(email: string, value: unknown) {
    localStorage.setItem(`${CACHE_KEY_PREFIX}${email}`, JSON.stringify(value));
    const updatedIndex = this._cacheIndex.filter((e) => e !== email);
    updatedIndex.push(email);

    if (updatedIndex.length > CACHE_SIZE) {
      const oldestKey = updatedIndex.shift();
      localStorage.removeItem(`${CACHE_KEY_PREFIX}${oldestKey}`);
    }

    localStorage.setItem(CACHE_INDEX_KEY, JSON.stringify(updatedIndex));
    this._cacheIndex = updatedIndex;
  }
}

export default new ParticipantProfileDataSource();
