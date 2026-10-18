import {
  discoveredSecurity,
  isLoopbackHost,
  needsCleartextConsent,
  repairStoredSecurity,
  securityForPort,
} from '../lib/connection-security';
import MailspringProviderSettings from '../lib/mailspring-provider-settings.json';

describe('connection security', function connectionSecuritySpecs() {
  it('maps the standard ports to TLS, never to none', () => {
    expect(securityForPort('imap', 993)).toEqual('SSL / TLS');
    expect(securityForPort('imap', 143)).toEqual('STARTTLS');
    expect(securityForPort('smtp', 465)).toEqual('SSL / TLS');
    expect(securityForPort('smtp', 587)).toEqual('STARTTLS');
    expect(securityForPort('smtp', 25)).toEqual('STARTTLS');
    expect(securityForPort('imap', 1143)).toBe(null);
  });

  it('never discovers none', () => {
    expect(discoveredSecurity('imap', 'SSL / TLS', 993)).toEqual('SSL / TLS');
    expect(discoveredSecurity('smtp', 'STARTTLS', 587)).toEqual('STARTTLS');
    for (const value of ['none', 'None', '', undefined]) {
      expect(discoveredSecurity('imap', value, 143)).toEqual('STARTTLS');
      expect(discoveredSecurity('imap', value, 993)).toEqual('SSL / TLS');
      expect(discoveredSecurity('imap', value, 1143)).toEqual('STARTTLS');
      expect(discoveredSecurity('smtp', value, 25)).toEqual('STARTTLS');
      expect(discoveredSecurity('smtp', value, 465)).toEqual('SSL / TLS');
    }
  });

  it('treats only loopback hosts as local', () => {
    for (const host of ['127.0.0.1', 'localhost', ' LocalHost ', '::1']) {
      expect(isLoopbackHost(host)).toBe(true);
    }
    for (const host of ['127.0.0.1.example.com', 'mail.example.com', '', undefined]) {
      expect(isLoopbackHost(host)).toBe(false);
    }
  });

  it('asks for consent for none on a remote host until it is given', () => {
    const settings: Record<string, any> = { imap_security: 'none', imap_host: 'imap.example.com' };
    expect(needsCleartextConsent(settings, 'imap')).toBe(true);
    settings.imap_allow_cleartext = ' IMAP.example.com ';
    expect(needsCleartextConsent(settings, 'imap')).toBe(false);
    // Given for another host, or a bare boolean: not consent for this one.
    settings.imap_host = 'imap.other.example';
    expect(needsCleartextConsent(settings, 'imap')).toBe(true);
    settings.imap_allow_cleartext = true;
    expect(needsCleartextConsent(settings, 'imap')).toBe(true);

    expect(needsCleartextConsent({ smtp_security: 'none', smtp_host: '127.0.0.1' }, 'smtp')).toBe(
      false
    );
    expect(
      needsCleartextConsent({ smtp_security: 'STARTTLS', smtp_host: 'smtp.example.com' }, 'smtp')
    ).toBe(false);
  });

  it('repairs a stored security the engine refuses, keeping valid ones', () => {
    const settings: Record<string, any> = {
      imap_security: 'None',
      imap_port: 143,
      smtp_security: '',
      smtp_port: 465,
    };
    expect(repairStoredSecurity(settings)).toBe(true);
    expect(settings.imap_security).toEqual('STARTTLS');
    expect(settings.smtp_security).toEqual('SSL / TLS');

    const valid = { imap_security: 'none', imap_port: 143, smtp_security: 'STARTTLS' };
    expect(repairStoredSecurity(valid)).toBe(false);
    expect(valid.imap_security).toEqual('none');
    const missing: Record<string, any> = { imap_port: 993, smtp_port: 2525 };
    repairStoredSecurity(missing);
    expect(missing).toEqual({
      imap_port: 993,
      imap_security: 'SSL / TLS',
      smtp_port: 2525,
      smtp_security: 'STARTTLS',
    });
  });

  it('ships no provider entry with cleartext', () => {
    const offenders = Object.entries(MailspringProviderSettings)
      .filter(([, entry]: [string, any]) => !entry.alias)
      .filter(
        ([, entry]: [string, any]) =>
          !['SSL / TLS', 'STARTTLS'].includes(entry.imap_security) ||
          !['SSL / TLS', 'STARTTLS'].includes(entry.smtp_security)
      )
      .map(([name]) => name);
    expect(offenders).toEqual([]);
  });
});
