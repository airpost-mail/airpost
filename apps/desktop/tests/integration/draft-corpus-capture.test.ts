// The desktop half of the draft pipeline's corpus.
//
// The engine builds drafts itself (`draft.create`, `draft.validate`, crates/tern-compose). Two
// checked-in files under crates/tern-compose are generated here and compared with what the desktop
// produces now, so a change on either side fails one of the two suites:
// - src/desktop_data.json: the lists the engine copies from the desktop (the sanitizer's tags and
//   attributes, the draft tracker rules, the user-agent stylesheet juice inlines with, the
//   salutations, name prefixes and suffixes, and the not-a-person address patterns).
// - tests/fixtures/draft_corpus.json: inputs and what the desktop makes of them: signatures,
//   plaintext wrapping and the send warnings. `crates/tern-compose/tests/corpus.rs` runs the same
//   inputs through the engine and compares. The quoting of replies and forwards, the reply,
//   forward and mailto drafts, the tracking pixels in drafts and the send warnings are only made
//   by the engine now (the desktop's copies were deleted), so their cases are carried over from
//   the file as they were last generated.
//
// After changing the desktop's drafting, regenerate both files with
//   TERN_WRITE_DRAFT_CORPUS=1 npm test -- --spec-file-pattern=draft-corpus-capture
// and run `cargo test -p tern-compose --test corpus`.
import fs from 'fs';
import path from 'path';
import { Account } from '../../src/data/models/account';
import { namePrefixes, nameSuffixes } from '../../src/data/models/contact';
import * as Utils from '../../src/data/models/utils';
import { AccountStore } from '../../src/data/stores/account';
import { AllowedAttributes, AllowedTags } from '../../src/mail/body/sanitize-transformer';
import { quoteTexts } from '../../src/intl/date-format';
import { wrapPlaintext } from '../../src/components/composer-editor/plaintext';
import userAgentStylesheet from '../../src/mail/body/chrome-user-agent-stylesheet';
import { applySignature } from '../../src/features/composer-signature/utils';
import { AdditionalRules } from '../../src/features/remove-tracking-pixels';
import { MailTrackerBlockerRules } from '../../src/features/remove-tracking-pixels/mail-tracker-blocker-rules';

const REPO = path.resolve(__dirname, '..', '..', '..', '..');
const DATA = path.join(REPO, 'crates', 'tern-compose', 'src', 'desktop_data.json');
const CORPUS = path.join(REPO, 'crates', 'tern-compose', 'tests', 'fixtures', 'draft_corpus.json');
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
      const FROZEN = JSON.parse(fs.readFileSync(CORPUS, 'utf8'));
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
        const { formatQuoteDate } = require('../../src/intl/date-format');
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

      // The engine builds the quoting, the drafts and mailto drafts and runs the send checks
      // alone now; their cases stay as the desktop last made them.
      for (const key of ['quoting', 'drafts', 'mailto', 'pixels', 'validate']) {
        corpus[key] = FROZEN[key];
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

      // Plaintext wrapping.
      for (const text of [
        `${'lorem ipsum '.repeat(10)}dolor`,
        `> ${'quoted words '.repeat(8)}end`,
        'short\nlines\n\nkept',
        `${'x'.repeat(100)} tail`,
      ]) {
        corpus.wrap.push({ text, desktop: wrapPlaintext(text) });
      }

      check(CORPUS, corpus);
    });
  });
});
