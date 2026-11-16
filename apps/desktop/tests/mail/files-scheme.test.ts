import fs from 'fs';
import os from 'os';
import path from 'path';
import { filesSchemeURL, resolveFilesSchemeURL } from '../../src/mail/files-scheme';

const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR4nGNikAQAACIAHF/uBd8AAAAASUVORK5CYII=',
  'base64'
);

describe('airpost-file: scheme', function () {
  let root: string;
  let filesDir: string;
  let outside: string;

  beforeEach(() => {
    root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'airpost-files-scheme-')));
    filesDir = path.join(root, 'files');
    fs.mkdirSync(path.join(filesDir, 'ab', 'cd', 'abcd1234'), { recursive: true });
    fs.writeFileSync(path.join(filesDir, 'ab', 'cd', 'abcd1234', 'image 1.png'), PNG);
    outside = path.join(root, 'secret.txt');
    fs.writeFileSync(outside, 'secret');
  });

  afterEach(() => {
    fs.rmSync(root, { recursive: true, force: true });
  });

  it('round-trips a file under the files directory', () => {
    const filePath = path.join(filesDir, 'ab', 'cd', 'abcd1234', 'image 1.png');
    const url = filesSchemeURL(filesDir, filePath);
    expect(url).toEqual('airpost-file:///ab/cd/abcd1234/image%201.png');
    expect(resolveFilesSchemeURL(filesDir, url)).toEqual(filePath);
  });

  it("escapes characters that would end a src='…' attribute", () => {
    const filePath = path.join(filesDir, 'ab', 'cd', 'abcd1234', `it's (1)$&.png`);
    fs.writeFileSync(filePath, PNG);
    const url = filesSchemeURL(filesDir, filePath);
    expect(url).not.toMatch(/['"()$&]/);
    expect(resolveFilesSchemeURL(filesDir, url)).toEqual(filePath);
  });

  it('builds no URL for a path outside the files directory', () => {
    expect(filesSchemeURL(filesDir, outside)).toBe(null);
    expect(filesSchemeURL(filesDir, filesDir)).toBe(null);
    expect(filesSchemeURL(filesDir, `${filesDir}-other/x.png`)).toBe(null);
  });

  it('rejects path traversal, encoded separators and other schemes', () => {
    for (const url of [
      'airpost-file:///../secret.txt',
      'airpost-file:///ab/../../secret.txt',
      'airpost-file:///%2e%2e/secret.txt',
      'airpost-file:///ab%2F..%2F..%2Fsecret.txt',
      'airpost-file:///ab%5C..%5C..%5Csecret.txt',
      `airpost-file:///${encodeURIComponent(outside)}`,
      'airpost-file:///ab/cd/abcd1234/image%201.png%00.txt',
      'airpost-file:///%E0%A4%A',
      'airpost-file:///',
      'airpost-file:///ab/cd/abcd1234',
      'airpost-file:///ab/cd/abcd1234/missing.png',
      `file://${outside}`,
      'airpost-asset://ab/cd/abcd1234/image%201.png',
    ]) {
      expect([url, resolveFilesSchemeURL(filesDir, url)]).toEqual([url, null]);
    }
  });

  it('rejects symlinks that lead out of the files directory', () => {
    fs.symlinkSync(outside, path.join(filesDir, 'ab', 'cd', 'abcd1234', 'link.png'));
    fs.symlinkSync(root, path.join(filesDir, 'ab', 'escape'));
    expect(resolveFilesSchemeURL(filesDir, 'airpost-file:///ab/cd/abcd1234/link.png')).toBe(null);
    expect(resolveFilesSchemeURL(filesDir, 'airpost-file:///ab/escape/secret.txt')).toBe(null);
  });

  it('follows symlinks that stay inside the files directory', () => {
    const target = path.join(filesDir, 'ab', 'cd', 'abcd1234', 'image 1.png');
    fs.symlinkSync(target, path.join(filesDir, 'ab', 'alias.png'));
    expect(resolveFilesSchemeURL(filesDir, 'airpost-file:///ab/alias.png')).toEqual(target);
  });

  describe('served by the main process', () => {
    // Spec windows get their own config dir, so use the one the main process serves from.
    const configDir = () => require('@electron/remote').getGlobal('application').configDirPath;
    const appFiles = () => path.join(configDir(), 'files');
    const dir = () => path.join(appFiles(), 'zz', 'zz', 'zzzz-files-scheme-spec');

    beforeEach(() => {
      fs.mkdirSync(dir(), { recursive: true });
      fs.writeFileSync(path.join(dir(), 'pixel.png'), PNG);
      fs.writeFileSync(path.join(configDir(), 'files-scheme-spec-outside.png'), PNG);
    });

    afterEach(() => {
      fs.rmSync(path.join(appFiles(), 'zz'), { recursive: true, force: true });
      fs.rmSync(path.join(configDir(), 'files-scheme-spec-outside.png'), {
        force: true,
      });
    });

    const loads = (src: string) =>
      new Promise<boolean>((resolve) => {
        const img = new Image();
        img.onload = () => resolve(true);
        img.onerror = () => resolve(false);
        img.src = src;
      });

    it('loads an attachment as an image and refuses anything outside files/', async () => {
      expect(await loads(filesSchemeURL(appFiles(), path.join(dir(), 'pixel.png')))).toBe(true);
      expect(await loads('airpost-file:///../files-scheme-spec-outside.png')).toBe(false);
      expect(await loads('airpost-file:///%2e%2e/files-scheme-spec-outside.png')).toBe(false);
    });
  });
});
