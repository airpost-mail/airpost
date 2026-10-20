import childProcess from 'child_process';
import crypto from 'crypto';
import fs from 'fs';
import path from 'path';
import { ipcRenderer } from 'electron';
import { localized } from './intl';

// Extensions the OS will run, install, mount or follow on double-click. Opening one asks first.
const MAC_EXTENSIONS = (
  'app action afploc applescript bash bundle cdr command configprofile csh definition dmg ' +
  'dmgpart dvdr fileloc ftploc imgpart inetloc iso jar kext ksh mobileconfig mpkg osas osax ' +
  'pkg pl plugin prefpane py rb saver scpt scptd service sh shortcut smi sparseimage terminal ' +
  'toast tool udif webloc widget workflow xpc zsh'
).split(' ');

// Covers Outlook's Level 1 list except certificates and data files (cer crt der pst tmp).
// library-ms, search-ms, searchconnector-ms and theme files leak NTLM hashes on open; py* run
// through the py launcher, which MotW doesn't stop.
const WINDOWS_EXTENSIONS = (
  'ade adp app appinstaller appref-ms appx appxbundle application bas bat bgi cab chm cmd cnt ' +
  'com cpl csh deskthemepack diagcab exe fxp gadget grp hlp hpj hta img inf ins isp its iso ' +
  'jar jnlp js jse ksh library-ms lnk mad maf mag mam maq mar mas mat mau mav maw mcf mda mdb ' +
  'mde mdt mdw mdz msc msh msh1 msh1xml msh2 msh2xml mshxml msi msix msixbundle msp mst msu ' +
  'ops osd pcd pif pl plg prf prg printerexport ps1 ps1xml ps2 ps2xml psc1 psc2 psd1 psdm1 ' +
  'psm1 py pyc pyo pyw pyz pyzw rdp reg scf scr sct search-ms searchconnector-ms ' +
  'settingcontent-ms shb shs theme themepack url vb vbe vbp vbs vhd vhdx vsmacros vsw webpnp ' +
  'website ws wsb wsc wsf wsh xbap xll xnk'
).split(' ');

// Windows types are here because a distro wine package runs them on double-click.
const LINUX_EXTENSIONS = (
  'appimage bash bat bin cmd com csh deb desktop exe flatpakref jar jnlp js ksh lnk msi msp ' +
  'pif pl py rb rpm run scr sh snap vbs zsh'
).split(' ');

const EXTENSIONS: { [platform: string]: Set<string> } = {
  darwin: new Set(MAC_EXTENSIONS),
  win32: new Set(WINDOWS_EXTENSIONS),
  linux: new Set(LINUX_EXTENSIONS),
};

// Bidi and zero-width characters let "invoice\u202Efdp.exe" display as "invoiceexe.pdf".
const INVISIBLE_CHARACTERS = /[\u200B-\u200F\u202A-\u202E\u2066-\u2069\uFEFF]/g;

function visibleName(fileName: string) {
  return path.basename(fileName || '').replace(INVISIBLE_CHARACTERS, '');
}

// The extension the OS acts on when it's on the platform's denylist, else null. Windows drops
// trailing dots and spaces, so "a.exe." and "a.exe " run as .exe.
export function executableExtension(
  fileName: string,
  platform: NodeJS.Platform = process.platform
): string | null {
  const name = visibleName(fileName)
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
  const name = visibleName(fileName).replace(/[. ]+$/, '');
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

// Handled by main/ipc/attachment-safety.ts.
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
// keepExisting leaves an existing quarantine attribute alone, so a file the user already approved
// isn't re-checked. Only pass it for files/: the engine renames files into place there, so any
// attribute came from Tern. A file overwritten in place keeps the old file's xattrs.
export async function markFileFromInternet(
  filePath: string,
  {
    keepExisting = false,
    platform = process.platform,
  }: { keepExisting?: boolean; platform?: NodeJS.Platform } = {}
): Promise<boolean> {
  try {
    if (platform === 'darwin') {
      if (keepExisting) {
        try {
          await execXattr(['-p', 'com.apple.quarantine', filePath]);
          return true;
        } catch {
          // No attribute yet.
        }
      }
      await execXattr(['-w', 'com.apple.quarantine', quarantineValue(), filePath]);
    } else if (platform === 'win32') {
      await fs.promises.writeFile(`${filePath}:Zone.Identifier`, ZONE_IDENTIFIER);
    }
    return true;
  } catch (err) {
    console.warn(`Could not mark ${filePath} as downloaded: ${err && err.message}`);
    return false;
  }
}

// PE (run by wine), ELF and #! scripts. xdg-open picks a handler by sniffing content when the
// extension is missing or unknown, so on Linux the name alone isn't enough.
const PROGRAM_SIGNATURES = [Buffer.from('MZ'), Buffer.from('\x7fELF', 'latin1'), Buffer.from('#!')];

export async function looksLikeProgram(filePath: string): Promise<boolean> {
  let handle: fs.promises.FileHandle;
  try {
    handle = await fs.promises.open(filePath, 'r');
    const head = Buffer.alloc(4);
    const { bytesRead } = await handle.read(head, 0, 4, 0);
    return PROGRAM_SIGNATURES.some(
      (sig) => bytesRead >= sig.length && head.subarray(0, sig.length).equals(sig)
    );
  } catch {
    return false;
  } finally {
    await handle?.close();
  }
}

// Resolves true when the file may be opened: it isn't on the denylist, or the user said so.
export async function confirmOpeningFile(
  filePath: string,
  platform: NodeJS.Platform = process.platform
): Promise<boolean> {
  const ext = executableExtension(filePath, platform);
  const notes: string[] = [];
  if (ext) {
    notes.push(
      localized(
        'This is a .%@ file, which can run programs on your computer. Only open it if you trust the sender and expected this file.',
        ext
      )
    );
  } else if (platform === 'linux' && (await looksLikeProgram(filePath))) {
    notes.push(
      localized(
        'This file is a program or script that can run on your computer. Only open it if you trust the sender and expected this file.'
      )
    );
  } else {
    return true;
  }
  const name = visibleName(filePath);
  const decoy = decoyExtension(filePath);
  if (ext && decoy && decoy !== ext) {
    notes.unshift(
      localized('The name makes it look like a .%@ file, but it is a .%@ file.', decoy, ext)
    );
  } else if (name !== path.basename(filePath || '')) {
    notes.unshift(localized('The name contains hidden characters that disguise its real type.'));
  }
  return ipcRenderer.invoke(ConfirmOpenChannel, localized('Open %@?', name), notes.join(' '));
}
