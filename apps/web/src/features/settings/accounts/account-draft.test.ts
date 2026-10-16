import { describe, expect, it } from 'vitest';
import { accountIdFor, buildAccount, formForPreset, validateForm } from './account-draft';
import { OTHER_PRESET, presetById, presetForEmail } from './presets';

describe('account draft', () => {
  it('computes the same id as the desktop', async () => {
    const settings = {
      imap_host: 'imap.fastmail.com',
      imap_username: 'maya@fastmail.com',
      smtp_host: 'smtp.fastmail.com',
      smtp_username: 'maya@fastmail.com',
      imap_port: 993,
    };
    // sha256('maya@fastmail.com' + JSON of the four critical fields), as the desktop's
    // idForAccount computes it with Node's crypto.
    expect(await accountIdFor('maya@fastmail.com', settings)).toBe('608bba5a');
  });

  it('fills servers from a preset and usernames from the email', () => {
    const f = formForPreset(presetById('gmail')!, { email: 'maya@gmail.com' });
    expect(f).toMatchObject({
      imapHost: 'imap.gmail.com',
      imapPort: '993',
      smtpHost: 'smtp.gmail.com',
      smtpSecurity: 'SSL / TLS',
      imapUsername: 'maya@gmail.com',
    });
  });

  it('guesses servers from the domain for Other IMAP', () => {
    const f = formForPreset(OTHER_PRESET, { email: 'me@okafor.studio' });
    expect(f.imapHost).toBe('imap.okafor.studio');
    expect(f.smtpHost).toBe('smtp.okafor.studio');
  });

  it('picks a preset by domain', () => {
    expect(presetForEmail('a@hotmail.com').id).toBe('office365');
    expect(presetForEmail('a@example.org').id).toBe('imap');
  });

  it('validates the required fields', () => {
    const f = formForPreset(OTHER_PRESET);
    expect(Object.keys(validateForm({ ...f, imapPort: '99999' }))).toEqual(
      expect.arrayContaining(['email', 'password', 'imapHost', 'imapPort', 'smtpHost']),
    );
  });

  it('builds an AccountJSON with the SMTP password defaulting to IMAP’s', async () => {
    const f = formForPreset(presetById('fastmail')!, { email: 'maya@fastmail.com' });
    const a = await buildAccount({ ...f, password: 'pw' }, { provider: 'fastmail' });
    expect(a).toMatchObject({
      __cls: 'Account',
      emailAddress: 'maya@fastmail.com',
      provider: 'fastmail',
      settings: { imap_password: 'pw', smtp_password: 'pw', imap_port: 993, smtp_port: 465 },
    });
    expect(a.id).toMatch(/^[0-9a-f]{8}$/);
    expect(a.aid).toBe(a.id);
  });
});
