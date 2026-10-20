import fs from 'fs';
import path from 'path';
import { nonNativeFilePathForDrop } from '../../../src/features/composer/view';

function transfer(data: { [type: string]: string }) {
  return { types: Object.keys(data), getData: (type: string) => data[type] } as any;
}

describe('Composer drops from inside the app', function () {
  const dir = () => path.join(AppEnv.getConfigDirPath(), 'files', 'zz', 'zz', 'composer-drop-spec');

  beforeEach(() => {
    fs.mkdirSync(dir(), { recursive: true });
    fs.writeFileSync(path.join(dir(), 'logo.png'), 'png');
  });

  afterEach(() => {
    fs.rmSync(dir(), { recursive: true, force: true });
  });

  it('attaches an inline image dragged from a message (a tern-file: URL)', () => {
    const uri = 'tern-file:///zz/zz/composer-drop-spec/logo.png';
    expect(nonNativeFilePathForDrop(transfer({ 'text/uri-list': uri }))).toEqual(
      fs.realpathSync(path.join(dir(), 'logo.png'))
    );
    expect(nonNativeFilePathForDrop(transfer({ 'text/uri-list': `${uri}\r\n` }))).toEqual(
      fs.realpathSync(path.join(dir(), 'logo.png'))
    );
  });

  it('ignores tern-file: URLs that name nothing under the files directory', () => {
    for (const uri of [
      'tern-file:///zz/zz/composer-drop-spec/missing.png',
      'tern-file:///zz/zz/composer-drop-spec/../../../../../etc/passwd',
    ]) {
      expect(nonNativeFilePathForDrop(transfer({ 'text/uri-list': uri }))).toBe(null);
    }
  });

  it('still takes file:// URIs and attachment chips', () => {
    expect(
      nonNativeFilePathForDrop(transfer({ 'text/uri-list': 'file:///tmp/a%20b.png' }))
    ).toEqual('/tmp/a b.png');
    expect(
      nonNativeFilePathForDrop(
        transfer({ 'text/mailspring-file-url': 'image/png:a.png:file:///tmp/a.png' })
      )
    ).toEqual('/tmp/a.png');
  });
});
