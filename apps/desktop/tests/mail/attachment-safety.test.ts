import childProcess from 'child_process';
import fs from 'fs';
import os from 'os';
import path from 'path';
import * as AttachmentSafety from '../../src/mail/attachment-safety';
import AttachmentStore from '../../src/data/stores/attachment';
import { File } from '../../src/data/models/file';

const {
  executableExtension,
  markFileFromInternet,
  confirmOpeningFile,
  quarantineValue,
  ZONE_IDENTIFIER,
} = AttachmentSafety;

const QUARANTINE =
  /^0081;[0-9a-f]{8};Airpost;[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}$/;

describe('attachment-safety', () => {
  describe('executableExtension', () => {
    it('matches the platform denylist case-insensitively', () => {
      expect(executableExtension('run.command', 'darwin')).toBe('command');
      expect(executableExtension('RUN.COMMAND', 'darwin')).toBe('command');
      expect(executableExtension('Setup.ExE', 'win32')).toBe('exe');
      expect(executableExtension('/files/ab/cd/id/payload.Lnk', 'win32')).toBe('lnk');
      expect(executableExtension('install.sh', 'linux')).toBe('sh');
    });

    it('goes by the last extension, so double extensions are caught', () => {
      expect(executableExtension('invoice.pdf.exe', 'win32')).toBe('exe');
      expect(executableExtension('Invoice.PDF.JS', 'win32')).toBe('js');
      expect(executableExtension('photo.jpg.app', 'darwin')).toBe('app');
      expect(executableExtension('invoice.exe.pdf', 'win32')).toBe(null);
    });

    it('sees through trailing dots, spaces and bidi overrides', () => {
      expect(executableExtension('invoice.exe.', 'win32')).toBe('exe');
      expect(executableExtension('invoice.exe  ', 'win32')).toBe('exe');
      // Displays as "invoiceexe.pdf".
      expect(executableExtension('invoice\u202Efdp.exe', 'win32')).toBe('exe');
    });

    it('leaves documents and other platforms alone', () => {
      expect(executableExtension('report.pdf', 'darwin')).toBe(null);
      expect(executableExtension('README', 'darwin')).toBe(null);
      expect(executableExtension('setup.exe', 'darwin')).toBe(null);
      expect(executableExtension('run.command', 'win32')).toBe(null);
      expect(executableExtension('', 'win32')).toBe(null);
    });

    it('covers types that run without an executable extension', () => {
      for (const ext of ['py', 'pyw', 'rdp', 'library-ms', 'search-ms', 'theme', 'msu', 'wsb']) {
        expect(executableExtension(`a.${ext}`, 'win32')).toBe(ext);
      }
      for (const ext of ['smi', 'dmgpart', 'toast', 'osas']) {
        expect(executableExtension(`a.${ext}`, 'darwin')).toBe(ext);
      }
      // Wine runs these on double-click.
      for (const ext of ['exe', 'msi', 'lnk']) {
        expect(executableExtension(`a.${ext}`, 'linux')).toBe(ext);
      }
    });
  });

  describe('markFileFromInternet on darwin', () => {
    it('writes com.apple.quarantine with xattr and no shell', async () => {
      const calls = [];
      spyOn(childProcess, 'execFile').andCallFake((cmd, args, cb) => {
        calls.push([cmd, args]);
        cb(args[0] === '-p' ? new Error('No such xattr: com.apple.quarantine') : null);
      });
      expect(await markFileFromInternet('/tmp/x/a.command', { platform: 'darwin' })).toBe(true);
      expect(calls.length).toBe(1);
      const [cmd, args] = calls[0];
      expect(cmd).toBe('/usr/bin/xattr');
      expect(args[0]).toBe('-w');
      expect(args[1]).toBe('com.apple.quarantine');
      expect(QUARANTINE.test(args[2])).toBe(true);
      expect(args[3]).toBe('/tmp/x/a.command');
    });

    it('keeps an existing quarantine attribute only when asked', async () => {
      const calls = [];
      spyOn(childProcess, 'execFile').andCallFake((cmd, args, cb) => {
        calls.push(args[0]);
        cb(null);
      });
      const platform = 'darwin';
      expect(await markFileFromInternet('/tmp/x/a.command', { platform, keepExisting: true })).toBe(
        true
      );
      expect(calls).toEqual(['-p']);
      calls.length = 0;
      expect(await markFileFromInternet('/tmp/x/a.command', { platform })).toBe(true);
      expect(calls).toEqual(['-w']);
    });

    it('logs and resolves false when xattr fails', async () => {
      spyOn(childProcess, 'execFile').andCallFake((cmd, args, cb) => cb(new Error('EPERM')));
      spyOn(console, 'warn');
      expect(await markFileFromInternet('/tmp/x/a.command', { platform: 'darwin' })).toBe(false);
      expect(console.warn).toHaveBeenCalled();
    });

    if (process.platform === 'darwin') {
      it('sets a real attribute on a temp file', async () => {
        const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'airpost-quarantine-'));
        const file = path.join(dir, 'a.command');
        fs.writeFileSync(file, '#!/bin/sh\n');
        try {
          expect(await markFileFromInternet(file)).toBe(true);
          const value = childProcess
            .execFileSync('/usr/bin/xattr', ['-p', 'com.apple.quarantine', file])
            .toString()
            .trim();
          expect(QUARANTINE.test(value)).toBe(true);
        } finally {
          fs.rmSync(dir, { recursive: true, force: true });
        }
      });

      it('replaces an approved attribute left by a file saved over', async () => {
        const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'airpost-quarantine-'));
        const source = path.join(dir, 'source.command');
        const dest = path.join(dir, 'run.command');
        fs.writeFileSync(source, '#!/bin/sh\necho new\n');
        fs.writeFileSync(dest, '#!/bin/sh\necho old\n');
        childProcess.execFileSync('/usr/bin/xattr', [
          '-w',
          'com.apple.quarantine',
          '00c1;65000000;Airpost;X',
          dest,
        ]);
        try {
          await AttachmentStore._writeToExternalPath(source, dest);
          const value = childProcess
            .execFileSync('/usr/bin/xattr', ['-p', 'com.apple.quarantine', dest])
            .toString()
            .trim();
          expect(QUARANTINE.test(value)).toBe(true);
        } finally {
          fs.rmSync(dir, { recursive: true, force: true });
        }
      });
    }
  });

  describe('markFileFromInternet on win32', () => {
    it('writes a Zone.Identifier stream with ZoneId=3', async () => {
      const write = spyOn(fs.promises, 'writeFile').andReturn(Promise.resolve());
      expect(
        await markFileFromInternet('C:\\Users\\a\\Downloads\\a.exe', { platform: 'win32' })
      ).toBe(true);
      expect(write).toHaveBeenCalledWith(
        'C:\\Users\\a\\Downloads\\a.exe:Zone.Identifier',
        ZONE_IDENTIFIER
      );
      expect(ZONE_IDENTIFIER).toBe('[ZoneTransfer]\r\nZoneId=3\r\n');
    });

    it('logs and resolves false on a volume without streams', async () => {
      spyOn(fs.promises, 'writeFile').andReturn(Promise.reject(new Error('ENOENT')));
      spyOn(console, 'warn');
      expect(await markFileFromInternet('E:\\a.exe', { platform: 'win32' })).toBe(false);
      expect(console.warn).toHaveBeenCalled();
    });
  });

  it('formats the quarantine value', () => {
    const value = quarantineValue(
      new Date(0x65000000 * 1000),
      'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee'
    );
    expect(value).toBe('0081;65000000;Airpost;AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE');
  });

  describe('confirmOpeningFile', () => {
    let invoke;
    beforeEach(() => {
      invoke = spyOn(require('electron').ipcRenderer, 'invoke').andReturn(Promise.resolve(false));
    });

    it('opens documents without asking', async () => {
      expect(await confirmOpeningFile('/files/report.pdf', 'win32')).toBe(true);
      expect(invoke).not.toHaveBeenCalled();
    });

    it('asks first and names the real extension', async () => {
      expect(await confirmOpeningFile('/files/invoice.pdf.exe', 'win32')).toBe(false);
      const [channel, message, detail] = invoke.calls[0].args;
      expect(channel).toBe(AttachmentSafety.ConfirmOpenChannel);
      expect(message).toContain('invoice.pdf.exe');
      expect(detail).toContain('.pdf');
      expect(detail).toContain('.exe');

      invoke.andReturn(Promise.resolve(true));
      expect(await confirmOpeningFile('/files/run.command', 'darwin')).toBe(true);
    });

    it('shows a name with bidi overrides as it really is', async () => {
      await confirmOpeningFile('/files/invoice\u202Efdp.exe', 'win32');
      const [, message, detail] = invoke.calls[0].args;
      expect(message).toBe('Open invoicefdp.exe?');
      expect(detail).toContain('hidden characters');
    });

    describe('on linux', () => {
      let dir: string;
      beforeEach(() => {
        dir = fs.mkdtempSync(path.join(os.tmpdir(), 'airpost-sniff-'));
      });
      afterEach(() => {
        fs.rmSync(dir, { recursive: true, force: true });
      });

      it('asks for programs whose name gives no hint', async () => {
        for (const [name, head] of [
          ['invoice', 'MZ\x90\x00'],
          ['invoice.dat', '\x7fELF'],
          ['notes', '#!/bin/sh\n'],
        ]) {
          const file = path.join(dir, name);
          fs.writeFileSync(file, Buffer.from(head, 'latin1'));
          expect(await confirmOpeningFile(file, 'linux')).toBe(false);
        }
        expect(invoke.calls.length).toBe(3);
      });

      it('opens other files without asking', async () => {
        const file = path.join(dir, 'notes');
        fs.writeFileSync(file, 'hello');
        expect(await confirmOpeningFile(file, 'linux')).toBe(true);
        expect(await confirmOpeningFile(path.join(dir, 'missing'), 'linux')).toBe(true);
        expect(invoke).not.toHaveBeenCalled();
      });
    });
  });

  describe('AttachmentStore', () => {
    const file = new File({ id: 'abcdef', filename: 'run.command' });
    let order: string[];

    beforeEach(() => {
      order = [];
      spyOn(AttachmentStore, '_prepareAndResolveFilePath').andReturn(
        Promise.resolve('/files/ab/cd/abcdef/run.command')
      );
      spyOn(require('electron').shell, 'openPath').andCallFake(() => {
        order.push('open');
        return Promise.resolve('');
      });
    });

    it('marks the file and asks before opening it', async () => {
      spyOn(AttachmentSafety, 'markFileFromInternet').andCallFake(async () => {
        order.push('mark');
        return true;
      });
      spyOn(AttachmentSafety, 'confirmOpeningFile').andCallFake(async () => {
        order.push('confirm');
        return true;
      });
      await AttachmentStore._fetchAndOpen(file);
      expect(order).toEqual(['mark', 'confirm', 'open']);
    });

    it('still asks, and does not open, when marking failed and the user cancels', async () => {
      spyOn(AttachmentSafety, 'markFileFromInternet').andReturn(Promise.resolve(false));
      spyOn(AttachmentSafety, 'confirmOpeningFile').andReturn(Promise.resolve(false));
      await AttachmentStore._fetchAndOpen(file);
      expect(AttachmentSafety.confirmOpeningFile).toHaveBeenCalled();
      expect(order).toEqual([]);
    });

    it('marks the copy after save-as', async () => {
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'airpost-save-'));
      const source = path.join(dir, 'source.command');
      const dest = path.join(dir, 'saved.command');
      fs.writeFileSync(source, '#!/bin/sh\n');
      const mark = spyOn(AttachmentSafety, 'markFileFromInternet').andReturn(Promise.resolve(true));
      try {
        await AttachmentStore._writeToExternalPath(source, dest);
        expect(fs.readFileSync(dest, 'utf8')).toBe('#!/bin/sh\n');
        expect(mark).toHaveBeenCalledWith(dest);
        expect(mark.calls[0].args.length).toBe(1);
      } finally {
        fs.rmSync(dir, { recursive: true, force: true });
      }
    });
  });
});
