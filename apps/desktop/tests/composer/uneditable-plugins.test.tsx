import React from 'react';
import ReactDOMServer from 'react-dom/server';
import { SignatureStore } from '../../src/data/stores/signature';
import { convertFromHTML, convertToHTML } from '../../src/components/composer-editor/conversion';
import UneditablePlugins, {
  UNEDITABLE_TYPE,
  withoutRemoteImages,
} from '../../src/components/composer-editor/uneditable-plugins';

const QUOTED =
  '<blockquote><p>Hi</p>' +
  '<img src="https://attacker.example/r/USER123.png" width="2" height="2">' +
  '<table background="https://attacker.example/bg.png"><tr>' +
  '<td style="background-image: url(https://attacker.example/td.png)">' +
  '<img src="cid:logo@x" srcset="https://attacker.example/2x.png 2x"></td></tr></table>' +
  '</blockquote>';

function renderEditorBlocks(html: string) {
  const value = convertFromHTML(html);
  const blocks = value.document
    .filterDescendants((n) => (n as any).type === UNEDITABLE_TYPE)
    .toArray();
  expect(blocks.length).toBeGreaterThan(0);
  return blocks
    .map((node) =>
      ReactDOMServer.renderToStaticMarkup(
        UneditablePlugins[0].renderNode(
          { node, attributes: {}, editor: null, children: null } as any,
          null,
          () => null
        ) as React.ReactElement
      )
    )
    .join('');
}

describe('Uneditable blocks and remote images', () => {
  let autoload: boolean;

  beforeEach(() => {
    autoload = false;
    const get = AppEnv.config.get.bind(AppEnv.config);
    spyOn(AppEnv.config, 'get').andCallFake((key: string) =>
      key === 'core.reading.autoloadImages' ? autoload : get(key)
    );
  });

  it('keeps remote image URLs out of the composer when remote images are off', () => {
    const markup = renderEditorBlocks(QUOTED);
    expect(markup).not.toMatch(/https?:/);
    expect(markup).toContain('cid:logo@x');
  });

  it('still sends the original images', () => {
    const html = convertToHTML(convertFromHTML(QUOTED));
    expect(html).toContain('https://attacker.example/r/USER123.png');
    expect(html).toContain('https://attacker.example/bg.png');
  });

  it('shows remote images when they are on', () => {
    autoload = true;
    expect(renderEditorBlocks(QUOTED)).toContain('https://attacker.example/r/USER123.png');
  });

  it("keeps the user's own signature images", () => {
    spyOn(SignatureStore, 'getSignatures').andReturn({
      sig: { id: 'sig', title: 'Me', body: '<img src="https://me.example/logo.png">' },
    });
    const html = withoutRemoteImages(
      '<img src="https://me.example/logo.png"><img src="https://attacker.example/x.png">'
    );
    expect(html).toContain('https://me.example/logo.png');
    expect(html).not.toContain('attacker.example');
  });
});
