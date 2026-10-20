// The desktop half of the draft pipeline's corpus (docs/desktop-client-plan.md stage 13).
//
// The engine builds drafts itself (`draft.create`, `draft.validate`, crates/ms-compose). Two
// checked-in files under crates/ms-compose are generated here and compared with what the desktop
// produces now, so a change on either side fails one of the two suites:
// - src/desktop_data.json: the lists the engine copies from the desktop (the sanitizer's tags and
//   attributes, the draft tracker rules, the user-agent stylesheet juice inlines with, the
//   salutations, name prefixes and suffixes, and the not-a-person address patterns).
// - tests/fixtures/draft_corpus.json: inputs and what the desktop makes of them: the quoting of
//   replies and forwards (sanitize and inline styles), signatures, tracking pixels in quotes,
//   mailto links, plaintext wrapping and the send warnings. `crates/ms-compose/tests/corpus.rs`
//   runs the same inputs through the engine and compares.
//
// After changing the desktop's drafting, regenerate both files with
//   TERN_WRITE_DRAFT_CORPUS=1 npm test -- --spec-file-pattern=draft-corpus-capture
// and run `cargo test -p ms-compose --test corpus`.
import fs from 'fs';
import path from 'path';
import { Account } from '../src/flux/models/account';
import { Message } from '../src/flux/models/message';
import { Contact } from '../src/flux/models/contact';
import { Thread } from '../src/flux/models/thread';
import { File } from '../src/flux/models/file';
import { namePrefixes, nameSuffixes } from '../src/flux/models/contact';
import * as Utils from '../src/flux/models/utils';
import { AccountStore } from '../src/flux/stores/account-store';
import ContactStore from '../src/flux/stores/contact-store';
import DraftFactory from '../src/flux/stores/draft-factory';
import { DraftEditingSession } from '../src/flux/stores/draft-editing-session';
import InlineStyleTransformer from '../src/services/inline-style-transformer';
import { AllowedAttributes, AllowedTags } from '../src/services/sanitize-transformer';
import { quoteTexts } from '../src/shared/date-format';
import { wrapPlaintext } from '../src/components/composer-editor/plaintext';
import userAgentStylesheet from '../src/chrome-user-agent-stylesheet-string';
import { applySignature } from '../features/composer-signature/lib/signature-utils';
import {
  AdditionalRules,
  removeTrackingPixelsFromDraft,
} from '../features/remove-tracking-pixels/lib/main';
import { MailTrackerBlockerRules } from '../features/remove-tracking-pixels/lib/mail-tracker-blocker-rules';
import TemplatesComposerExtension from '../plugins/composer-templates/lib/template-composer-extension';

const REPO = path.resolve(__dirname, '..', '..', '..');
const DATA = path.join(REPO, 'crates', 'ms-compose', 'src', 'desktop_data.json');
const CORPUS = path.join(REPO, 'crates', 'ms-compose', 'tests', 'fixtures', 'draft_corpus.json');
const WRITE = !!process.env.TERN_WRITE_DRAFT_CORPUS;
const TZ = 'America/Chicago';

const ACCOUNTS = [
  {
    id: 'acc1',
    name: 'Ann Owner',
    emailAddress: 'ann@example.com',
    provider: 'imap',
    aliases: ['Ann at Work <ann@work.example>', 'Support Desk <help@example.com>'],
    autoaddress: { type: 'bcc', value: '' },
  },
  {
    id: 'acc2',
    name: 'Ann Personal',
    emailAddress: 'ann.p@example.org',
    provider: 'imap',
    aliases: [],
    defaultAlias: 'Annie <ann.p@example.org>',
    autoaddress: { type: 'cc', value: 'Archive <archive@example.org>, log@example.org' },
  },
];

const SIGNATURES = {
  sig1: { id: 'sig1', title: 'Work', body: '<div>Ann Owner<br/>Example Inc.</div>' },
  sig2: { id: 'sig2', title: 'Personal', body: '<b>-- Annie</b>' },
};
const DEFAULT_SIGNATURES = { 'ann@example.com': 'sig1', 'ann.p@example.org': 'sig2' };

// Received at 2023-11-14 22:13:20 UTC.
const DATE = 1700000000;

const BODIES: { [name: string]: string } = {
  simple: '<p>Hello Ann,</p><p>See you <b>tomorrow</b>.</p>',
  styled:
    '<html><head><style>.big { font-size: 20px; color: #333 } p { margin: 0 } #x { color: red }</style></head>' +
    '<body><p class="big">Big text</p><p id="x">Red</p><div>plain</div></body></html>',
  'style-import':
    '<style>@import url("https://tracker.example/x.css"); td { padding: 4px }</style>' +
    '<table><tr><td>cell</td></tr></table>',
  scripts:
    '<p onclick="steal()">Hi</p><script>alert(1)</script><iframe src="https://evil.example"></iframe>' +
    '<a href="javascript:alert(1)">bad</a> <a href="https://ok.example/" target="_blank">ok</a>' +
    '<div data-secret="1" title="t">data</div><form action="https://f.example"><input name="a"></form>',
  images:
    '<p>Logo <img src="cid:logo123" alt="logo"> and <img src="https://cdn.example/a.png" width="100"></p>',
  pixels:
    '<p>News</p><img src="https://list-manage.com/track/open.php?u=1" width="1" height="1">' +
    '<img src="https://cdn.example/hidden.gif" style="display:none">' +
    '<img src="https://cdn.example/photo.jpg" width="300">',
  quoted:
    '<div>Top reply</div><div class="gmail_quote"><div class="gmail_quote_attribution">On Mon, Bob wrote:</div>' +
    '<blockquote class="gmail_quote" style="margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex;">' +
    '<p>Older text</p></blockquote></div>',
  entities: '<p>Caf&eacute; &amp; cr&egrave;me &lt;3 — “quotes” 😀</p><p>&nbsp;</p>',
  newsletter:
    '<!DOCTYPE html><html><head><meta http-equiv="refresh" content="0;url=https://x.example">' +
    '<title>News</title><style type="text/css"><!-- .btn { background: #06c; color: #fff } --></style>' +
    '<link rel="stylesheet" href="https://x.example/s.css"></head><body bgcolor="#eee">' +
    '<table width="600" align="center" cellpadding="0"><tr><td class="btn">Click</td></tr></table></body></html>',
  'attach-mention': '<p>Hi Carol,</p><p>I attached the report.</p>',
};

const PLAINTEXT_BODY = 'Hello Ann,\n\n> quoted earlier\n> more\n\nThanks & bye <3';

function contact(name: string, email: string) {
  return new Contact({ name, email });
}

function received(name: string, body: string, extra: Partial<Message> = {}) {
  return new Message({
    id: `msg-${name}`,
    accountId: 'acc1',
    headerMessageId: `${name}@sender.example`,
    threadId: 'thread-1',
    subject: 'Quarterly numbers',
    from: [contact('Bob Sender', 'bob@sender.example')],
    to: [contact('Ann Owner', 'ann@example.com'), contact('Carol', 'carol@sender.example')],
    cc: [contact('', 'dan@sender.example')],
    bcc: [],
    replyTo: [],
    date: new Date(DATE * 1000),
    body,
    plaintext: false,
    draft: false,
    files: [],
    ...extra,
  } as any);
}

const pick = (draft: Message) => {
  const j = draft.toJSON() as any;
  const people = (list: any[]) => (list || []).map((c) => ({ name: c.name || '', email: c.email }));
  return {
    accountId: j.aid,
    from: people(j.from),
    to: people(j.to),
    cc: people(j.cc),
    bcc: people(j.bcc),
    subject: j.subject,
    body: draft.body,
    plaintext: !!j.plaintext,
    threadId: j.threadId || '',
    rthMsgId: j.rthMsgId || '',
    fwdMsgId: j.fwdMsgId || '',
    files: (j.files || []).map((f) => f.id),
  };
};

const messageJSON = (m: Message) => ({ ...(m.toJSON() as any), body: m.body });

// The built-in prepareNewDraft hooks, as the composer-signature and remove-tracking-pixels
// packages register them (in that order).
function prepareNewDraft(draft: Message) {
  if (!draft.plaintext) {
    const from = draft.from && draft.from[0];
    const sig = from ? SIGNATURES[DEFAULT_SIGNATURES[from.email]] : null;
    if (sig) draft.body = applySignature(draft.body, sig);
  }
  removeTrackingPixelsFromDraft(draft);
  return draft;
}

function check(file: string, value: unknown) {
  const text = `${JSON.stringify(value, null, 2)}\n`;
  if (WRITE) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, text);
    return;
  }
  const existing = fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : '';
  if (existing !== text) {
    // Jasmine 1.3 prints the whole strings; say what to do first.
    console.warn(
      `${path.relative(REPO, file)} is out of date; regenerate it (see the spec header)`
    );
  }
  expect(existing).toEqual(text);
}

describe('Draft pipeline corpus (desktop half)', function corpusSpec() {
  let savedAccounts;
  let config: { [key: string]: unknown };

  beforeEach(() => {
    const store = AccountStore as any;
    savedAccounts = store._accounts;
    store._accounts = ACCOUNTS.map((a) => new Account(a as any));
    store._caches = {};
    spyOn(ContactStore, 'searchContacts').andCallFake(() => Promise.resolve([]));
    spyOn(AppEnv.keymaps, 'getIsAltKeyDown').andReturn(false);
    // juice as the main process runs it (application.ts, 'inline-style-parse').
    spyOn(InlineStyleTransformer, 'run').andCallFake((html: string) => {
      if (!InlineStyleTransformer._requiresProcessing(html)) return Promise.resolve(html);
      const prepared = InlineStyleTransformer._prepareHTMLForInlineStyling(html);
      try {
        return Promise.resolve(require('juice')(prepared));
      } catch (err) {
        return Promise.resolve(prepared);
      }
    });
    config = {
      'core.composing.html': true,
      'core.workspace.use24HourClock': false,
      'core.sending.defaultAccountIdForSend': 'acc1',
    };
    const realGet = AppEnv.config.get.bind(AppEnv.config);
    spyOn(AppEnv.config, 'get').andCallFake((key: string) =>
      key in config ? config[key] : realGet(key)
    );
  });

  afterEach(() => {
    (AccountStore as any)._accounts = savedAccounts;
    (AccountStore as any)._caches = {};
  });

  it('keeps the lists the engine copies in step', () => {
    check(DATA, {
      sanitizer: { tags: AllowedTags, attributes: AllowedAttributes },
      trackerRules: [...Object.values(MailTrackerBlockerRules), ...Object.values(AdditionalRules)]
        .flat()
        .filter((rule) => !rule.startsWith('/')),
      userAgentStylesheet,
      salutations: Utils.commonlyCapitalizedSalutations,
      nonHumanEmailTerms: Utils.NonHumanEmailTerms,
      namePrefixes: Object.keys(namePrefixes),
      nameSuffixes: Object.keys(nameSuffixes),
    });
  });

  it('keeps the corpus in step', () => {
    waitsForPromise(async () => {
      const thread = new Thread({
        id: 'thread-1',
        accountId: 'acc1',
        subject: 'Quarterly numbers',
      });
      const corpus: any = {
        tz: TZ,
        accounts: ACCOUNTS,
        signatures: SIGNATURES,
        defaultSignatures: DEFAULT_SIGNATURES,
        quoteTexts: quoteTexts('en', 'h12', 'en-US'),
        quoteTexts24: quoteTexts('en', 'h23', 'en-US'),
        quoteTextsDe: quoteTexts('de', 'h23', 'de-DE'),
        dates: [],
        quoting: [],
        drafts: [],
        signaturesApplied: [],
        pixels: [],
        mailto: [],
        wrap: [],
        validate: [],
      };

      for (const [locale, cycle] of [
        ['en-US', 'h12'],
        ['en-US', 'h23'],
        ['de-DE', 'h23'],
        ['ja-JP', 'h12'],
      ] as const) {
        const { formatQuoteDate } = require('../src/shared/date-format');
        for (const at of [DATE, 978310800, 1704067140]) {
          corpus.dates.push({
            at,
            locale,
            cycle,
            texts: quoteTexts(locale.split('-')[0], cycle, locale),
            desktop: formatQuoteDate(new Date(at * 1000), { cycle, locale }),
          });
        }
      }

      for (const [name, body] of Object.entries(BODIES)) {
        const m = received(name, body);
        corpus.quoting.push({
          name,
          message: messageJSON(m),
          html: true,
          desktop: await DraftFactory.prepareBodyForQuoting(m),
        });
      }
      const plain = received('plaintext', PLAINTEXT_BODY, { plaintext: true } as any);
      corpus.quoting.push({
        name: 'plaintext',
        message: messageJSON(plain),
        html: true,
        desktop: await DraftFactory.prepareBodyForQuoting(plain),
      });

      // Drafts: replies, reply-all, forwards, new drafts, as DraftStore saves them.
      const draftCase = async (
        name: string,
        kind: string,
        make: () => Promise<Message>,
        input: any
      ) => {
        const draft = prepareNewDraft(await make());
        corpus.drafts.push({ name, kind, ...input, desktop: pick(draft) });
      };
      for (const name of ['simple', 'styled', 'pixels', 'quoted', 'images']) {
        const m = received(name, BODIES[name]);
        await draftCase(
          `reply-${name}`,
          'reply',
          () => DraftFactory.createDraftForReply({ message: m, thread, type: 'reply' }),
          { message: messageJSON(m), html: true }
        );
      }
      {
        const m = received('reply-all', BODIES.simple, {
          cc: [
            contact('', 'dan@sender.example'),
            contact('Ann', 'ANN@example.com'),
            contact('Bob again', 'bob+x@sender.example'),
          ],
        } as any);
        await draftCase(
          'reply-all',
          'reply-all',
          () => DraftFactory.createDraftForReply({ message: m, thread, type: 'reply-all' }),
          { message: messageJSON(m), html: true }
        );
        const toAlias = received('to-alias', BODIES.simple, {
          to: [contact('Support Desk', 'help@example.com')],
        } as any);
        await draftCase(
          'reply-to-alias',
          'reply',
          () => DraftFactory.createDraftForReply({ message: toAlias, thread, type: 'reply' }),
          { message: messageJSON(toAlias), html: true }
        );
        const replyTo = received('reply-to', BODIES.simple, {
          replyTo: [contact('List', 'list@lists.example')],
        } as any);
        await draftCase(
          'reply-with-reply-to',
          'reply-all',
          () => DraftFactory.createDraftForReply({ message: replyTo, thread, type: 'reply-all' }),
          { message: messageJSON(replyTo), html: true }
        );
        const fromMe = received('from-me', BODIES.simple, {
          from: [contact('Ann at Work', 'ann@work.example')],
          to: [contact('Erin', 'erin@else.example')],
          cc: [contact('Frank', 'frank@else.example'), contact('Ann Owner', 'ann@example.com')],
        } as any);
        await draftCase(
          'reply-all-from-me',
          'reply-all',
          () => DraftFactory.createDraftForReply({ message: fromMe, thread, type: 'reply-all' }),
          { message: messageJSON(fromMe), html: true }
        );
        const fwdSubject = received('fwd-subject', BODIES.simple, {
          subject: 'FWD: Quarterly',
        } as any);
        await draftCase(
          'reply-to-forward',
          'reply',
          () => DraftFactory.createDraftForReply({ message: fwdSubject, thread, type: 'reply' }),
          { message: messageJSON(fwdSubject), html: true }
        );
      }
      for (const name of ['simple', 'newsletter']) {
        const m = received(`fwd-${name}`, BODIES[name], {
          files: [
            new File({ id: 'file1', filename: 'report.pdf', accountId: 'acc1', size: 3 } as any),
          ],
        } as any);
        await draftCase(
          `forward-${name}`,
          'forward',
          () => DraftFactory.createDraftForForward({ message: m, thread }),
          { message: messageJSON(m), html: true }
        );
      }
      await draftCase(
        'new',
        'new',
        () =>
          DraftFactory.createDraft({ to: [contact('Gail', 'gail@else.example')], subject: 'Hi' }),
        {
          fields: { to: [{ name: 'Gail', email: 'gail@else.example' }], subject: 'Hi' },
          html: true,
        }
      );
      await draftCase(
        'new-acc2-autocc',
        'new',
        () =>
          DraftFactory.createDraft({
            accountId: 'acc2',
            from: [AccountStore.accountForId('acc2').defaultMe()],
          }),
        {
          fields: { aid: 'acc2', from: [{ name: 'Annie', email: 'ann.p@example.org' }] },
          html: true,
        }
      );

      // Plaintext composing (core.composing.html off).
      config['core.composing.html'] = false;
      {
        const m = received('plain-reply', BODIES.simple);
        await draftCase(
          'reply-plaintext-of-html',
          'reply',
          () => DraftFactory.createDraftForReply({ message: m, thread, type: 'reply' }),
          { message: messageJSON(m), html: false }
        );
        await draftCase(
          'reply-plaintext-of-plain',
          'reply',
          () => DraftFactory.createDraftForReply({ message: plain, thread, type: 'reply' }),
          { message: messageJSON(plain), html: false }
        );
        await draftCase(
          'new-plaintext',
          'new',
          () => DraftFactory.createDraft({ body: `${'word '.repeat(30)}end` }),
          { fields: { body: `${'word '.repeat(30)}end` }, html: false }
        );
      }
      config['core.composing.html'] = true;

      // mailto: links.
      for (const url of [
        'mailto:someone@example.net',
        'mailto:someone@example.net?subject=Hello%20there&body=Line%201%0ALine%202&cc=a%40b.example',
        'mailto:?to=x@y.example,z@w.example&bcc=hidden@y.example&subject=Hi',
        'mailto:Some%20One%20%3Cone@example.net%3E?body=https://github.com/x/issues?utf8=&q=is%3Aopen&subject=Bug',
        'mailto:user%40example.net',
      ]) {
        const draft = prepareNewDraft(await DraftFactory.createDraftForMailto(url));
        corpus.mailto.push({ url, html: true, desktop: pick(draft) });
      }

      // Signatures.
      for (const [name, body] of Object.entries({
        empty: '<br/>',
        text: '<div>Hi</div>',
        'two-brs': '<div>Hi</div><br/><br/>',
        'before-quote': '<div>Hi</div><br/><div class="gmail_quote">quoted</div>',
        'replace-existing':
          '<div>Hi</div><signature id="sig2"><b>-- Annie</b></signature><div class="gmail_quote">q</div>',
      })) {
        for (const sig of [SIGNATURES.sig1, null]) {
          corpus.signaturesApplied.push({
            name: `${name}-${sig ? sig.id : 'none'}`,
            body,
            signature: sig,
            desktop: applySignature(body, sig),
          });
        }
      }

      // Tracking pixels in a draft.
      for (const body of [
        BODIES.pixels,
        BODIES.images,
        '<img src="https://t.yesware.com/x" ><picture><source srcset="https://app.yesware.com/t/1 1x, https://cdn.example/b.png 2x"></picture>',
        '<img src="data:image/png;base64,AAA" width="1" height="1"><input type="image" src="https://cdn.example/i.png" style="width:0">',
      ]) {
        const d = new Message({ body } as any);
        removeTrackingPixelsFromDraft(d);
        corpus.pixels.push({ body, desktop: d.body });
      }

      // Plaintext wrapping.
      for (const text of [
        `${'lorem ipsum '.repeat(10)}dolor`,
        `> ${'quoted words '.repeat(8)}end`,
        'short\nlines\n\nkept',
        `${'x'.repeat(100)} tail`,
      ]) {
        corpus.wrap.push({ text, desktop: wrapPlaintext(text) });
      }

      // Send warnings (DraftEditingSession and the templates extension).
      const validateCase = (name: string, fields: any) => {
        const draft = new Message({
          accountId: 'acc1',
          headerMessageId: `${name}@example.com`,
          from: [contact('Ann Owner', 'ann@example.com')],
          to: [],
          cc: [],
          bcc: [],
          subject: 'Subject',
          body: '<p>Hello</p>',
          files: [],
          draft: true,
          ...fields,
        } as any);
        const json = { ...(draft.toJSON() as any), body: draft.body };
        const session = new DraftEditingSession(draft.headerMessageId, draft);
        const { recipientErrors, recipientWarnings } = session.validateDraftRecipients();
        const { miscErrors, miscWarnings } = session.validateDraftForSending();
        miscWarnings.push(
          ...TemplatesComposerExtension.warningsForSending({ draft: session.draft() })
        );
        session.teardown();
        corpus.validate.push({
          name,
          draft: json,
          desktop: { recipientErrors, recipientWarnings, miscErrors, miscWarnings },
        });
      };
      const carol = [contact('Carol Jones', 'carol@sender.example')];
      validateCase('ok', { to: carol, body: '<p>Hi Carol,</p>' });
      validateCase('no-recipients', {});
      validateCase('invalid-recipient', { to: [contact('', 'not-an-address')] });
      validateCase('blank-subject', { to: carol, subject: '' });
      validateCase('attach-mentioned', { to: carol, body: BODIES['attach-mention'] });
      validateCase('attach-in-quote-only', {
        to: carol,
        body: '<p>Hi</p><div class="gmail_quote"><blockquote>see attached</blockquote></div>',
      });
      validateCase('attach-with-file', {
        to: carol,
        body: BODIES['attach-mention'],
        files: [new File({ id: 'f', filename: 'a.pdf' } as any)],
      });
      validateCase('salutation-mismatch', { to: carol, body: '<p>Hi Dave,</p><p>text</p>' });
      validateCase('salutation-first-name', { to: carol, body: '<p>Hey Carol!</p>' });
      validateCase('salutation-family', { to: carol, body: '<p>Hi Mom,</p>' });
      validateCase('salutation-unnamed', {
        to: [contact('', 'carol@sender.example')],
        body: '<p>Hi Dave,</p>',
      });
      validateCase('salutation-nonhuman', {
        to: [contact('Billing', 'noreply@shop.example')],
        body: '<p>Dear Sir,</p>',
      });
      validateCase('unsendable-from', {
        to: carol,
        from: [contact('Stranger', 'stranger@else.example')],
      });
      validateCase('from-alias', {
        to: carol,
        from: [contact('Support Desk', 'help@example.com')],
      });
      validateCase('empty-template', {
        to: carol,
        body: '<p>Hi</p><p><code class="var empty">name</code></p>',
      });
      validateCase('signature-attach', {
        to: carol,
        body: '<p>Hello</p><signature id="sig1">attachment policy</signature>',
      });

      check(CORPUS, corpus);
    });
  });
});
