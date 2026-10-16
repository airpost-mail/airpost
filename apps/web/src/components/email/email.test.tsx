import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { EmailFrame } from './EmailFrame';
import { MessageBody } from './MessageBody';
import { buildSrcDoc, frameCsp, hasRemoteContent, prepareHtml } from './sanitize';

describe('email sanitising', () => {
  it('detects remote images and backgrounds', () => {
    expect(hasRemoteContent('<img src="https://x.example/a.png">')).toBe(true);
    expect(hasRemoteContent('<td style="background:url(//x.example/b.png)">')).toBe(true);
    expect(hasRemoteContent('<img src="data:image/png;base64,AAA">')).toBe(false);
  });

  it('blocks remote images in the CSP unless allowed', () => {
    expect(frameCsp(false)).not.toContain('https:');
    expect(frameCsp(true)).toContain('https:');
    expect(frameCsp(false)).toContain("default-src 'none'");
  });

  it('strips scripts, meta refresh and base tags, and resolves cid', () => {
    const out = prepareHtml(
      '<meta http-equiv="refresh" content="0;url=https://x.example"><base href="https://x.example/"><script>alert(1)</script><img src="cid:logo@x">',
      (cid) => (cid === 'logo@x' ? 'blob:local/1' : null),
    );
    expect(out).not.toMatch(/refresh|<base|<script/i);
    expect(out).toContain('src="blob:local/1"');
  });

  it('strips resource hints and other ways to reach a server without images', () => {
    const out = prepareHtml(
      '<link rel="preconnect" href="https://u1.t.example"><link rel="dns-prefetch" href="//u2.t.example">' +
        '<meta http-equiv="x-dns-prefetch-control" content="on"><iframe src="https://x.example"></iframe>' +
        '<a href="https://ok.example" ping="https://t.example/p">ok</a><style>p{color:red}</style>',
    );
    expect(out).not.toMatch(/<link|http-equiv|<iframe|ping=|t\.example/i);
    expect(out).toContain('href="https://ok.example"');
    expect(out).toContain('<style>');
  });

  it('puts the CSP in the frame document', () => {
    expect(buildSrcDoc('<p>x</p>', { allowRemoteImages: false })).toContain(
      'Content-Security-Policy',
    );
  });
});

describe('EmailFrame', () => {
  it('renders a sandboxed iframe without allow-scripts', () => {
    render(<EmailFrame html="<p>Hello</p>" />);
    const frame = screen.getByTitle('Message body');
    expect(frame.getAttribute('sandbox')).toBe('allow-same-origin');
    expect(frame.getAttribute('srcdoc')).toContain('<p>Hello</p>');
  });
});

describe('MessageBody', () => {
  it('renders plain text without a frame', () => {
    render(<MessageBody body={'Hi <b>there</b>'} plaintext />);
    expect(screen.getByText('Hi <b>there</b>')).toBeInTheDocument();
    expect(screen.queryByTitle('Message body')).toBeNull();
  });

  it('offers to load remote images', () => {
    render(<MessageBody body='<img src="https://x.example/a.png">' />);
    const frame = screen.getByTitle('Message body');
    expect(frame.getAttribute('srcdoc')).not.toContain('img-src data: blob: https:');
    fireEvent.click(screen.getByRole('button', { name: 'Load images' }));
    expect(screen.getByTitle('Message body').getAttribute('srcdoc')).toContain('https: http:');
  });
});
