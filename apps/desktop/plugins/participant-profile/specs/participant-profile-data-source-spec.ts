import { CapabilityStore, Integrations } from 'mailspring-exports';
import ParticipantProfileDataSource from '../lib/participant-profile-data-source';

const contact = { email: 'person@company.example.test', name: 'Person' };

describe('ParticipantProfileDataSource', () => {
  let sent: any[];
  let reply: any;

  beforeEach(() => {
    localStorage.clear();
    sent = [];
    reply = {};
    Integrations.keySource = { getPassword: async () => 'synthetic-pdl-key' };
    Integrations.setTransport({
      available: true,
      send: async (method, params) => {
        sent.push({ method, params });
        if (reply instanceof Error || reply.key) throw reply;
        return reply;
      },
    });
  });

  afterEach(() => {
    Integrations.setTransport(null);
    Integrations.keySource = null;
  });

  it('makes no lookup while contact enrichment is unavailable', async () => {
    expect(await ParticipantProfileDataSource.find(contact)).toEqual({});
    expect(sent.length).toBe(0);
  });

  it('looks people up with integrations.enrich and keeps matches', async () => {
    AppEnv.config.set('core.integrations.enrichment', { enabled: true });
    spyOn(CapabilityStore, 'isAvailable').andCallFake((f) => f === 'contact-enrichment');
    reply = {
      person: { name: 'Person', employment: { title: 'CTO', name: 'Company' } },
      company: { name: 'Company', domain: 'company.example.test' },
    };

    const profile = await ParticipantProfileDataSource.find(contact);
    expect(profile).toEqual(reply);
    expect(sent).toEqual([
      {
        method: 'integrations.enrich',
        params: {
          email: contact.email,
          config: { provider: 'peopledatalabs', apiKey: 'synthetic-pdl-key' },
        },
      },
    ]);

    // The second view comes from localStorage.
    expect(await ParticipantProfileDataSource.find(contact)).toEqual(reply);
    expect(sent.length).toBe(1);
  });

  it('does not keep a no-match or an error', async () => {
    AppEnv.config.set('core.integrations.enrichment', { enabled: true });
    spyOn(CapabilityStore, 'isAvailable').andCallFake((f) => f === 'contact-enrichment');
    spyOn(console, 'warn');

    expect(await ParticipantProfileDataSource.find(contact)).toEqual({});
    expect(ParticipantProfileDataSource.hasCache(contact.email)).toBe(false);

    reply = { key: 'auth', message: 'rejected', retryable: false, status: 401 };
    expect(await ParticipantProfileDataSource.find(contact)).toEqual({});
    expect(ParticipantProfileDataSource.hasCache(contact.email)).toBe(false);
    expect(console.warn).toHaveBeenCalledWith('Contact lookup failed: auth');
    expect(sent.length).toBe(2);
  });
});
