import childProcess from 'child_process';
import crypto from 'crypto';
import fs from 'fs';
import path from 'path';
import { ipcRenderer } from 'electron';
import { localized } from './intl';

// Extensions the OS will run, install, mount or follow on double-click. Opening one asks first.
const MAC_EXTENSIONS = (
  'app action afploc applescript bash bundle cdr command configprofile csh definition dmg ' +
  'fileloc ftploc inetloc iso jar kext ksh mobileconfig mpkg osax pkg pl plugin prefpane py ' +
  'rb saver scpt scptd service sh shortcut sparseimage terminal tool webloc widget workflow ' +
  'xpc zsh'
).split(' ');

const WINDOWS_EXTENSIONS = (
  'appref-ms appx appxbundle application bat cab chm cmd com cpl diagcab exe gadget hlp hta ' +
  'img inf ins isp iso jar js jse lnk msc msi msix msixbundle msp mst pif ps1 ps1xml ps2 ' +
  'ps2xml psc1 psc2 psd1 psm1 reg scf scr sct settingcontent-ms shb shs url vb vbe vbs vhd ' +
  'vhdx website ws wsc wsf wsh xbap xll'
).split(' ');

const LINUX_EXTENSIONS =
  'appimage bash bin csh deb desktop flatpakref jar ksh pl py rb rpm run sh snap zsh'.split(' ');

const EXTENSIONS: { [platform: string]: Set<string> } = {
  darwin: new Set(MAC_EXTENSIONS),
  win32: new Set(WINDOWS_EXTENSIONS),
  linux: new Set(LINUX_EXTENSIONS),
};

// Bidi and zero-width characters let "invoice\u202Efdp.exe" display as "invoiceexe.pdf".
const INVISIBLE_CHARACTERS = /[\u200B-\u200F\u202A-\u202E\u2066-\u2069\uFEFF]/g;

// The extension the OS acts on when it's on the platform's denylist, else null. Windows drops
// trailing dots and spaces, so "a.exe." and "a.exe " run as .exe.
export function executableExtension(
  fileName: string,
  platform: NodeJS.Platform = process.platform
): string | null {
  const name = path
    .basename(fileName || '')
    .replace(INVISIBLE_CHARACTERS, '')
    .replace(/[. ]+$/, '')
    .toLowerCase();
  const dot = name.lastIndexOf('.');
  if (dot === -1) {
    return null;
  }
  const ext = name.slice(dot + 1);
  const denylist = EXTENSIONS[platform] || EXTENSIONS.linux;
  return denylist.has(ext) ? ext : null;
}

// For "invoice.pdf.exe", the extension the name pretends to have ("pdf").
function decoyExtension(fileName: string): string | null {
  const name = path
    .basename(fileName || '')
    .replace(INVISIBLE_CHARACTERS, '')
    .replace(/[. ]+$/, '');
  const stem = name.slice(0, name.lastIndexOf('.'));
  const ext = path.extname(stem).slice(1);
  return ext && /^[a-z0-9]{1,5}$/i.test(ext) ? ext.toLowerCase() : null;
}

// Value format: flags;hex epoch;agent;event UUID. 0x0001 marks the file as downloaded, so
// Gatekeeper checks it on first open.
export function quarantineValue(now: Date = new Date(), uuid: string = crypto.randomUUID()) {
  const epoch = Math.floor(now.getTime() / 1000).toString(16);
  return `0081;${epoch};Tern;${uuid.toUpperCase()}`;
}

// Handled by browser/attachment-safety-ipc.ts.
export const ConfirmOpenChannel = 'attachment-safety:confirm-open';

export const ZONE_IDENTIFIER = '[ZoneTransfer]\r\nZoneId=3\r\n';

function execXattr(args: string[]) {
  return new Promise<void>((resolve, reject) => {
    childProcess.execFile('/usr/bin/xattr', args, (err) => (err ? reject(err) : resolve()));
  });
}

// Tag a file as downloaded from the internet: com.apple.quarantine on macOS, a Zone.Identifier
// stream (Mark-of-the-Web) on Windows, so Gatekeeper, SmartScreen and Office Protected View
// check it. Never rejects: a failure is logged and the caller carries on with its own checks.
export async function markFileFromInternet(
  filePath: string,
  platform: NodeJS.Platform = process.platform
): Promise<boolean> {
  try {
    if (platform === 'darwin') {
      try {
        // Keep an existing attribute, so a file the user already approved isn't re-checked.
        await execXattr(['-p', 'com.apple.quarantine', filePath]);
        return true;
      } catch {
        await execXattr(['-w', 'com.apple.quarantine', quarantineValue(), filePath]);
      }
    } else if (platform === 'win32') {
      await fs.promises.writeFile(`${filePath}:Zone.Identifier`, ZONE_IDENTIFIER);
    }
    return true;
  } catch (err) {
    console.warn(`Could not mark ${filePath} as downloaded: ${err && err.message}`);
    return false;
  }
}

// Resolves true when the file may be opened: it isn't on the denylist, or the user said so.
export async function confirmOpeningFile(
  filePath: string,
  platform: NodeJS.Platform = process.platform
): Promise<boolean> {
  const ext = executableExtension(filePath, platform);
  if (!ext) {
    return true;
  }
  const decoy = decoyExtension(filePath);
  let detail = localized(
    'This is a .%@ file, which can run programs on your computer. Only open it if you trust the sender and expected this file.',
    ext
  );
  if (decoy && decoy !== ext) {
    detail = `${localized(
      'The name makes it look like a .%@ file, but it is a .%@ file.',
      decoy,
      ext
    )} ${detail}`;
  }
  return ipcRenderer.invoke(
    ConfirmOpenChannel,
    localized('Open %@?', path.basename(filePath)),
    detail
  );
}
