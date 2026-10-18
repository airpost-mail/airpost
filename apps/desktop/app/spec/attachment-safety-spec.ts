import childProcess from 'child_process';
import fs from 'fs';
import os from 'os';
import path from 'path';
import * as AttachmentSafety from '../src/attachment-safety';
import { AttachmentStore, File } from 'mailspring-exports';

const {
  executableExtension,
  markFileFromInternet,
  confirmOpeningFile,
  quarantineValue,
  ZONE_IDENTIFIER,
} = AttachmentSafety;

const QUARANTINE =
  /^0081;[0-9a-f]{8};Tern;[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}$/;

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
  });

  describe('markFileFromInternet on darwin', () => {
    it('writes com.apple.quarantine with xattr and no shell', async () => {
      const calls = [];
      spyOn(childProcess, 'execFile').andCallFake((cmd, args, cb) => {
        calls.push([cmd, args]);
        cb(args[0] === '-p' ? new Error('No such xattr: com.apple.quarantine') : null);
      });
      expect(await markFileFromInternet('/tmp/x/a.command', 'darwin')).toBe(true);
      expect(calls.length).toBe(2);
      const [cmd, args] = calls[1];
      expect(cmd).toBe('/usr/bin/xattr');
      expect(args[0]).toBe('-w');
      expect(args[1]).toBe('com.apple.quarantine');
      expect(QUARANTINE.test(args[2])).toBe(true);
      expect(args[3]).toBe('/tmp/x/a.command');
    });

    it('keeps an existing quarantine attribute', async () => {
      const calls = [];
      spyOn(childProcess, 'execFile').andCallFake((cmd, args, cb) => {
        calls.push(args[0]);
        cb(null);
      });
      expect(await markFileFromInternet('/tmp/x/a.command', 'darwin')).toBe(true);
      expect(calls).toEqual(['-p']);
    });

    it('logs and resolves false when xattr fails', async () => {
      spyOn(childProcess, 'execFile').andCallFake((cmd, args, cb) => cb(new Error('EPERM')));
      spyOn(console, 'warn');
      expect(await markFileFromInternet('/tmp/x/a.command', 'darwin')).toBe(false);
      expect(console.warn).toHaveBeenCalled();
    });

    if (process.platform === 'darwin') {
      it('sets a real attribute on a temp file', async () => {
        const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-quarantine-'));
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
    }
  });

  describe('markFileFromInternet on win32', () => {
    it('writes a Zone.Identifier stream with ZoneId=3', async () => {
      const write = spyOn(fs.promises, 'writeFile').andReturn(Promise.resolve());
      expect(await markFileFromInternet('C:\\Users\\a\\Downloads\\a.exe', 'win32')).toBe(true);
      expect(write).toHaveBeenCalledWith(
        'C:\\Users\\a\\Downloads\\a.exe:Zone.Identifier',
        ZONE_IDENTIFIER
      );
      expect(ZONE_IDENTIFIER).toBe('[ZoneTransfer]\r\nZoneId=3\r\n');
    });

    it('logs and resolves false on a volume without streams', async () => {
      spyOn(fs.promises, 'writeFile').andReturn(Promise.reject(new Error('ENOENT')));
      spyOn(console, 'warn');
      expect(await markFileFromInternet('E:\\a.exe', 'win32')).toBe(false);
      expect(console.warn).toHaveBeenCalled();
    });
  });

  it('formats the quarantine value', () => {
    const value = quarantineValue(
      new Date(0x65000000 * 1000),
      'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee'
    );
    expect(value).toBe('0081;65000000;Tern;AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE');
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
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tern-save-'));
      const source = path.join(dir, 'source.command');
      const dest = path.join(dir, 'saved.command');
      fs.writeFileSync(source, '#!/bin/sh\n');
      const mark = spyOn(AttachmentSafety, 'markFileFromInternet').andReturn(Promise.resolve(true));
      try {
        await AttachmentStore._writeToExternalPath(source, dest);
        expect(fs.readFileSync(dest, 'utf8')).toBe('#!/bin/sh\n');
        expect(mark).toHaveBeenCalledWith(dest);
      } finally {
        fs.rmSync(dir, { recursive: true, force: true });
      }
    });
  });
});
