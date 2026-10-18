/* eslint no-irregular-whitespace: 0 */
import fs from 'fs';
import { isTrackingImage, removeTrackingPixels } from '../lib/main';
import { Message } from 'mailspring-exports';

const readFixture = (name) => {
  return fs.readFileSync(`${__dirname}/fixtures/${name}`).toString().trim();
};

const strip = (body: string, isFromMe = false) => {
  const message = { body, accountId: '1234', isFromMe: () => isFromMe };
  removeTrackingPixels(message as Message);
  return message.body;
};

describe('TrackingPixelsExtension', function trackingPixelsExtension() {
  it("should splice all tracking pixels from emails I've sent", () => {
    expect(strip(readFixture('a-before.txt'), true)).toEqual(readFixture('a-after.txt'));
  });

  it('splices Mailspring read receipts whoever sent them', () => {
    expect(strip(readFixture('b-before.txt'))).toEqual(readFixture('b-after.txt'));
  });

  it('strips known trackers from mail sent by others', () => {
    const trackers = [
      '<img src="https://sdr.salesloft.com/email_trackers/8c8bea88/open.gif" alt="">',
      '<img src="https://shop.us1.list-manage.com/track/open.php?u=1&amp;id=2" border="0">',
      '<img src="https://t.yesware.com/t/abc/def/spacer.gif" width="300" height="20">',
      '<img src="https://mailtrack.io/trace/mail/abc.png">',
      '<img src="https://t.hubspotemail.net/e1t/o/5/abc" style="width:20px">',
      "<img src='https://u123.ct.sendgrid.net/wf/open?upn=abc'>",
      '<img src=https://www.google-analytics.com/collect?v=1&tid=UA-1>',
    ];
    for (const tag of trackers) {
      expect(strip(`<p>Hi</p>${tag}<p>Bye</p>`)).toEqual('<p>Hi</p><p>Bye</p>');
    }
  });

  it('leaves ordinary remote, inline and data images alone', () => {
    const kept = [
      '<img src="https://cdn.example.com/hero.png" width="600" height="300" alt="Sale">',
      '<img src="https://cdn.example.com/logo.png">',
      '<img src="https://cdn.example.com/rule.png" width="600" height="1">',
      '<img src="https://cdn.example.com/rule.png" width="100%" height="1">',
      '<img src="cid:logo@example.com" width="1" height="1">',
      '<img src="data:image/gif;base64,R0lGODlhAQABAAAAACw=" style="display:none">',
    ];
    for (const tag of kept) {
      expect(isTrackingImage(tag)).toBe(false);
      expect(strip(`<p>Hi</p>${tag}`)).toEqual(`<p>Hi</p>${tag}`);
    }
  });

  it('strips hidden, zero-sized and 1x1 remote images from mail sent by others', () => {
    const hidden = [
      '<img src="https://mail.example.net/o/abc.gif" width="1" height="1">',
      '<img src="https://mail.example.net/o/abc.gif" width="1px" height="1px" border="0">',
      '<img src="https://mail.example.net/o/abc.gif" width=1 height=1>',
      '<img src="https://mail.example.net/o/abc.gif" width="1">',
      '<img src="https://mail.example.net/o/abc.gif" width="0" height="0">',
      '<img src="https://mail.example.net/o/abc.gif" height="0" width="200">',
      '<img src="https://mail.example.net/o/abc.gif" style="width: 1px; height: 1px !important">',
      '<img src="https://mail.example.net/o/abc.gif" width="600" style="width:0;height:0">',
      '<img src="https://mail.example.net/o/abc.gif" style="display: none">',
      '<img src="https://mail.example.net/o/abc.gif" style="visibility:hidden" width="40">',
      '<img src="https://mail.example.net/o/abc.gif" hidden>',
      '<img alt="a > b" src="//mail.example.net/o/abc.gif" width="1" height="1" />',
      '<IMG SRC="HTTP://mail.example.net/o/abc.gif" WIDTH="1" HEIGHT="1">',
    ];
    for (const tag of hidden) {
      expect(isTrackingImage(tag)).toBe(true);
      expect(strip(`<p>Hi</p>${tag}<p>Bye</p>`)).toEqual('<p>Hi</p><p>Bye</p>');
    }
  });
});
