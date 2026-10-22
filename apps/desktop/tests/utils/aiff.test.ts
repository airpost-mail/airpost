import fs from 'fs';
import { pathToFileURL } from 'url';
import { aiffToWav } from '../../src/utils/aiff';
import {
  macSystemSoundNames,
  macSystemSoundPath,
  playsFromNotificationCenter,
  systemSoundDirs,
  systemSoundPath,
} from '../../src/platform/system-sounds';
import { isFileURLWithin } from '../../src/utils/file-url';

// 44100 as an 80-bit extended float.
const RATE_44100 = Buffer.from([0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]);

function aiff(samples: number[], { form = 'AIFF', bits = 16 } = {}) {
  const comm = Buffer.alloc(26);
  comm.write('COMM', 0, 'ascii');
  comm.writeUInt32BE(18, 4);
  comm.writeUInt16BE(1, 8); // mono
  comm.writeUInt32BE(samples.length, 10);
  comm.writeUInt16BE(bits, 14);
  RATE_44100.copy(comm, 16);
  const pcm = Buffer.alloc(samples.length * 2);
  samples.forEach((s, i) => pcm.writeInt16BE(s, i * 2));
  const ssnd = Buffer.alloc(16);
  ssnd.write('SSND', 0, 'ascii');
  ssnd.writeUInt32BE(8 + pcm.length, 4);
  const body = Buffer.concat([Buffer.from(form, 'ascii'), comm, ssnd, pcm]);
  const head = Buffer.alloc(8);
  head.write('FORM', 0, 'ascii');
  head.writeUInt32BE(body.length, 4);
  return Buffer.concat([head, body]);
}

describe('aiffToWav', function () {
  it('rewrites big-endian PCM as a little-endian WAV', () => {
    const wav = aiffToWav(aiff([1, -2, 0x1234])) as Buffer;
    expect(wav.toString('ascii', 0, 4)).toBe('RIFF');
    expect(wav.toString('ascii', 8, 16)).toBe('WAVEfmt ');
    expect(wav.readUInt16LE(22)).toBe(1);
    expect(wav.readUInt32LE(24)).toBe(44100);
    expect(wav.readUInt16LE(34)).toBe(16);
    expect(wav.readUInt32LE(40)).toBe(6);
    expect([0, 1, 2].map((i) => wav.readInt16LE(44 + i * 2))).toEqual([1, -2, 0x1234]);
  });

  it('returns null for AIFF-C, other files and odd sample sizes', () => {
    expect(aiffToWav(aiff([1], { form: 'AIFC' }))).toBe(null);
    expect(aiffToWav(aiff([1], { bits: 12 }))).toBe(null);
    expect(aiffToWav(Buffer.from('RIFF0000WAVE', 'ascii'))).toBe(null);
    expect(aiffToWav(Buffer.alloc(0))).toBe(null);
  });

  if (process.platform === 'darwin' && fs.existsSync('/System/Library/Sounds/Glass.aiff')) {
    it('converts the macOS system sounds', () => {
      const wav = aiffToWav(fs.readFileSync('/System/Library/Sounds/Glass.aiff')) as Buffer;
      expect(wav.readUInt16LE(34)).toBe(24);
      expect(wav.readUInt32LE(24)).toBe(48000);
    });
  }
});

describe('system sounds', function () {
  it('takes only plain macOS sound names', () => {
    expect(macSystemSoundPath('../../etc/passwd')).toBe(null);
    expect(macSystemSoundPath('NoSuchSound')).toBe(null);
    expect(macSystemSoundPath(undefined)).toBe(null);
  });

  const MAIL = '/System/Applications/Mail.app/Contents/Resources';
  if (process.platform === 'darwin' && fs.existsSync(`${MAIL}/New Mail.aiff`)) {
    it("maps events to Apple Mail's sounds on macOS, with the picked new-mail sound", () => {
      expect(systemSoundPath('new-mail')).toBe(`${MAIL}/New Mail.aiff`);
      expect(systemSoundPath('new-mail', 'NoSuchSound')).toBe(`${MAIL}/New Mail.aiff`);
      expect(systemSoundPath('new-mail', 'Ping')).toBe('/System/Library/Sounds/Ping.aiff');
      expect(systemSoundPath('hit-send')).toBe(`${MAIL}/Mail Sent.aiff`);
      expect(systemSoundPath('send')).toBe(false);
      expect(macSystemSoundNames()[0]).toBe('New Mail');
    });

    it('plays only /System/Library/Sounds from NotificationCenter, and both dirs in place', () => {
      expect(playsFromNotificationCenter(`${MAIL}/New Mail.aiff`)).toBe(false);
      expect(playsFromNotificationCenter('/System/Library/Sounds/Ping.aiff')).toBe(true);
      const url = pathToFileURL(`${MAIL}/Mail Sent.aiff`).href;
      expect(isFileURLWithin(url, systemSoundDirs())).toBe(true);
    });

    it("converts Mail's sounds", () => {
      for (const name of ['New Mail', 'Mail Sent']) {
        const wav = aiffToWav(fs.readFileSync(`${MAIL}/${name}.aiff`)) as Buffer;
        expect(wav.toString('ascii', 0, 4)).toBe('RIFF');
        expect(wav.readUInt16LE(34)).toBe(24);
      }
    });
  }
});
