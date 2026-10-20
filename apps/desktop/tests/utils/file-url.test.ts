import path from 'path';
import { pathToFileURL } from 'url';
import { isFileURLWithin, isHostedFileURL } from '../../src/utils/file-url';

describe('file URL guard', function () {
  it('flags file: URLs that name a host or a UNC path', () => {
    for (const url of [
      'file://attacker.example/share/p.png',
      'FILE://attacker.example/share/p.png',
      'file:////attacker.example/share/p.png',
      'file://192.168.0.1/c$/x',
      'file:\\\\attacker.example\\share\\p.png',
    ]) {
      expect(`${url}: ${isHostedFileURL(url)}`).toEqual(`${url}: true`);
    }
  });

  it('leaves local file: URLs and other schemes alone', () => {
    for (const url of [
      'file:///Users/me/x.png',
      'file:///C:/Users/me/x.png',
      'file://localhost/Users/me/x.png',
      'file://C:/Users/me/x.png',
      'https://attacker.example/share/p.png',
      'tern-file:///ab/cd/x.png',
      'data:image/png;base64,AAAA',
    ]) {
      expect(`${url}: ${isHostedFileURL(url)}`).toEqual(`${url}: false`);
    }
  });

  it('a protocol-relative src in a file: document resolves to a hosted file: URL', () => {
    const base = pathToFileURL(
      path.resolve('/desktop/src/mail/quick-preview/renderer.html')
    ).toString();
    expect(isHostedFileURL(new URL('//attacker.example/share/p.png', base).toString())).toBe(true);
    expect(isHostedFileURL(new URL('\\\\attacker.example\\share\\p.png', base).toString())).toBe(
      true
    );
  });

  it('limits file: URLs to the given roots', () => {
    const root = path.resolve('/tern/files');
    const inside = pathToFileURL(path.join(root, 'ab', 'cd', 'x.pdf')).toString();
    expect(isFileURLWithin(inside, [root])).toBe(true);
    expect(isFileURLWithin(`${inside}?q=1#page=2`, [root])).toBe(true);
    expect(
      isFileURLWithin(pathToFileURL(path.resolve('/tern/files-other/x')).toString(), [root])
    ).toBe(false);
    expect(isFileURLWithin(pathToFileURL(path.resolve('/etc/passwd')).toString(), [root])).toBe(
      false
    );
    expect(isFileURLWithin(`${pathToFileURL(root)}/../secret`, [root])).toBe(false);
    expect(isFileURLWithin('file://attacker.example/tern/files/x', [root])).toBe(false);
    expect(isFileURLWithin('https://example.com/tern/files/x', [root])).toBe(false);
  });
});
