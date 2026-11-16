// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/*
.vcf files in and out of the contacts list. The engine reads and writes the vCards
(`contacts.import`, `contacts.export`); this keeps the dialogs and the files.
*/
import fs from 'fs';
import { Contact } from '../../data/models/contact';
import { AccountStore } from '../../data/stores/account';
import { localized } from '../../intl/localize';
import { exportContacts, importContacts } from '../../engine/client/pim';
import { Dialogs } from '../../platform/app-api';
import { writePrivateTempFile } from '../../utils/private-temp';

function showInfo(title: string, message: string) {
  Dialogs.showMessageBox({ type: 'info', title, message, buttons: [localized('OK')] }, 'none');
}

/** Show a native save dialog and write the given contacts to a .vcf file. */
export function exportContactsToFile(contacts: Contact[]) {
  if (contacts.length === 0) return;
  const defaultName =
    contacts.length === 1 ? `${contacts[0].name || 'contact'}.vcf` : 'contacts.vcf';
  AppEnv.showSaveDialog({ defaultPath: defaultName }, async (filePath) => {
    if (!filePath) return;
    try {
      await fs.promises.writeFile(filePath, await exportContacts(contacts), 'utf-8');
    } catch (err) {
      console.error('VCF export failed:', err);
      AppEnv.showErrorDialog(
        localized(
          'The contacts could not be exported: %@',
          err instanceof Error ? err.message : String(err)
        )
      );
    }
  });
}

/**
 * Writes the given contacts to a temporary .vcf file and returns its path, for the DownloadURL
 * of a drag to the desktop. The drag needs the path at once: the file starts with the CardDAV
 * cards as stored and is rewritten with the engine's export, Google and mail contacts included,
 * long before the drop.
 */
export function writeContactsToTempVCF(contacts: Contact[]): string {
  const safeName =
    contacts.length === 1
      ? (contacts[0].name || 'contact').replace(/[^a-z0-9 ]/gi, '_').trim() || 'contact'
      : 'contacts';
  const stored = contacts
    .map((c) => (c.info && 'vcf' in c.info ? c.info.vcf : ''))
    .filter(Boolean)
    .join('');
  const filePath = writePrivateTempFile(`${safeName}.vcf`, stored);
  exportContacts(contacts)
    .then((vcf) => fs.promises.writeFile(filePath, vcf, 'utf-8'))
    .catch((err) => console.warn('Could not export the dragged contacts:', err));
  return filePath;
}

/**
 * Imports the vCards in already-resolved file paths into the given account, as new contacts.
 * Used by both the file-dialog import and the drag-drop handler.
 */
export async function importContactsFromPaths(filePaths: string[], accountId: string) {
  const account = AccountStore.accountForId(accountId);
  if (!account) return;

  let imported = 0;
  let failed = false;
  for (const filePath of filePaths) {
    try {
      const vcf = await fs.promises.readFile(filePath, 'utf-8');
      const result = await importContacts({ accountId, vcf });
      imported += result.contactIds.length;
    } catch (err) {
      failed = true;
      console.error(`VCF import: failed to import ${filePath}:`, err);
    }
  }

  if (imported === 0) {
    showInfo(
      failed ? localized('Import Failed') : localized('No Contacts Found'),
      failed
        ? localized('The selected file(s) could not be imported.')
        : localized('No valid contacts were found in the selected file(s).')
    );
  }
}

/** Show a native open-file dialog and import contacts from the selected .vcf file(s). */
export function importContactsFromFile(accountId: string) {
  if (!AccountStore.accountForId(accountId)) return;
  AppEnv.showOpenDialog(
    {
      title: localized('Import VCards'),
      filters: [{ name: 'VCard Files', extensions: ['vcf', 'vcard'] }],
      properties: ['openFile', 'multiSelections'],
    },
    (filePaths) => {
      if (!filePaths || filePaths.length === 0) return;
      importContactsFromPaths(filePaths, accountId);
    }
  );
}
