// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

// Writes account_ids.json: `idForAccount` copied from
// apps/desktop/features/onboarding/lib/onboarding-helpers.ts as it was before the
// engine derived ids, run on the cases below. Run with `node account_ids.js` from this directory.
const crypto = require('crypto');
const fs = require('fs');

function idForAccount(emailAddress, connectionSettings) {
  const settingsThatCouldChangeMailContents = {
    imap_username: connectionSettings.imap_username,
    imap_host: connectionSettings.imap_host,
    smtp_username: connectionSettings.smtp_username,
    smtp_host: connectionSettings.smtp_host,
  };
  const idString = `${emailAddress}${JSON.stringify(settingsThatCouldChangeMailContents)}`;
  return crypto.createHash('sha256').update(idString, 'utf8').digest('hex').substr(0, 8);
}

const cases = [
  ['plain', 'me@example.com', { imap_username: 'me@example.com', imap_host: 'imap.example.com', smtp_username: 'me@example.com', smtp_host: 'smtp.example.com', imap_port: 993, imap_security: 'SSL / TLS' }],
  ['gmail', 'ben@gmail.com', { imap_username: 'ben@gmail.com', imap_host: 'imap.gmail.com', smtp_username: 'ben@gmail.com', smtp_host: 'smtp.gmail.com', refresh_client_id: 'x' }],
  ['o365 shared mailbox', 'shared@contoso.com', { imap_username: 'shared@contoso.com', imap_host: 'outlook.office365.com', smtp_username: 'owner@contoso.com', smtp_host: 'smtp.office365.com' }],
  ['no settings', 'nohosts@example.com', {}],
  ['missing and null', 'partial@example.com', { imap_host: 'imap.example.com', smtp_host: null }],
  ['escapes and non-ascii', 'Jöhn.Dœ@exämple.org', { imap_username: 'jöhn "q" \\ dœ /', imap_host: 'imap.exämple.org', smtp_username: 'tab\there\nnl\r\u0001\u001f\u007f\b\f', smtp_host: 'smtp.exämple.org' }],
  ['astral and html', 'emoji@example.com', { imap_username: '😀 user', imap_host: 'mail.example.com', smtp_username: '</script> ', smtp_host: 'mail.example.com' }],
  ['email-without-domain username', 'jane@me.com', { imap_username: 'jane', imap_host: 'imap.mail.me.com', smtp_username: 'jane@me.com', smtp_host: 'smtp.mail.me.com' }],
  ['number', 'num@example.com', { imap_username: 'num', imap_host: 'h', smtp_username: 7, smtp_host: 's' }],
];

const out = cases.map(([name, email, settings]) => ({ name, email, settings, id: idForAccount(email, settings) }));
fs.writeFileSync('account_ids.json', JSON.stringify(out, null, 2) + '\n');
