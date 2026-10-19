// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

// The contacts package's files and saves on the engine (contacts.*). The vCards it reads and
// writes are crates/ms-pim's tests and the desktop contact corpus.
import fs from 'fs';
import os from 'os';
import path from 'path';
import { AccountStore } from 'mailspring-exports';
import { exportContacts, saveContact } from '../src/engine-pim';
import {
  importContactsFromPaths,
  writeContactsToTempVCF,
} from '../internal_packages/contacts/lib/ContactFiles';
import { fakePimEngine, flush } from './fake-pim-engine';

const CARD = 'BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada\r\nEND:VCARD\r\n';

describe('contacts on the engine', function () {
  let engine: ReturnType<typeof fakePimEngine>;
  let file: string;
  let info: jasmine.Spy;

  beforeEach(function () {
    engine = fakePimEngine();
    engine.answers['contacts.export'] = (params) => ({
      vcf: params.ids.map((id: string) => CARD.replace('Ada', id)).join(''),
    });
    engine.answers['contacts.import'] = () => ({ contactIds: ['c-new'] });
    engine.answers['contacts.update'] = () => ({});
    engine.answers['contacts.create'] = () => ({});
    spyOn(AccountStore, 'accountForId').andReturn({ id: 'a1' });
    info = spyOn(require('@electron/remote').dialog, 'showMessageBox');
    file = path.join(os.tmpdir(), `contact-files-spec-${process.pid}.vcf`);
  });

  afterEach(function () {
    if (fs.existsSync(file)) fs.unlinkSync(file);
  });

  it('exports each account its contacts together, as one file', async function () {
    const vcf = await exportContacts([
      { id: 'c1', accountId: 'a1' },
      { id: 'c2', accountId: 'a2' },
      { id: 'c3', accountId: 'a1' },
    ]);
    expect(engine.callsTo('contacts.export')).toEqual([
      { accountId: 'a1', ids: ['c1', 'c3'] },
      { accountId: 'a2', ids: ['c2'] },
    ]);
    expect(vcf.match(/BEGIN:VCARD/g).length).toBe(3);
  });

  it('imports the files into the account through the engine', async function () {
    fs.writeFileSync(file, CARD);
    await importContactsFromPaths([file], 'a1');
    expect(engine.callsTo('contacts.import')).toEqual([{ accountId: 'a1', vcf: CARD }]);
    expect(info).not.toHaveBeenCalled();
  });

  it('says so when a file held no contacts', async function () {
    engine.answers['contacts.import'] = () => ({ contactIds: [] });
    fs.writeFileSync(file, '');
    await importContactsFromPaths([file], 'a1');
    expect(info).toHaveBeenCalled();
    expect(info.mostRecentCall.args[0].title).toBe('No Contacts Found');
  });

  it('writes the stored cards for a drag at once, then the engine export', async function () {
    const dragged: any = [
      { id: 'c1', accountId: 'a1', name: 'Ada', info: { vcf: CARD, href: '/c1.vcf' } },
    ];
    const written = writeContactsToTempVCF(dragged);
    file = written;
    expect(fs.readFileSync(written, 'utf-8')).toBe(CARD);
    const exported = CARD.replace('Ada', 'c1');
    await flush();
    for (let i = 0; i < 50 && fs.readFileSync(written, 'utf-8') !== exported; i++) {
      await new Promise((r) => setImmediate(r));
    }
    expect(fs.readFileSync(written, 'utf-8')).toBe(exported);
  });

  it('saves an existing contact as an update and a new one as a create', async function () {
    await saveContact({ accountId: 'a1', contactId: 'c1', data: { name: 'Ada' } });
    await saveContact({ accountId: 'a1', data: { name: 'Bo' } });
    expect(engine.callsTo('contacts.update')).toEqual([
      { accountId: 'a1', contactId: 'c1', patch: { name: 'Ada' } },
    ]);
    expect(engine.callsTo('contacts.create')).toEqual([{ accountId: 'a1', data: { name: 'Bo' } }]);
  });
});
