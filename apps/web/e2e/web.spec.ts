// The web client against tern-gateway and a `serve` engine on the harness's fake mail servers
// (e2e/stack.py). One browser page walks through the whole run in order; each step leaves a
// screenshot in <work>/shots.
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { expect, test, type BrowserContext, type Frame, type Page } from '@playwright/test';

interface StackInfo {
  work: string;
  origin: string;
  pair: { code: string; url: string };
  control: string;
  accounts: Record<'alice' | 'bob', { id: string; email: string }>;
  inlinePngSha256: string;
  reportPdfSha256: string;
}

interface SmtpMessage {
  server: string;
  recipients: string[];
  subject: string | null;
  text: string | null;
  parts: { type: string; filename: string | null; size: number; sha256: string }[];
}

interface ImapMessage {
  uid: number;
  subject: string;
  flags: string[];
}

const info: StackInfo = JSON.parse(fs.readFileSync(process.env.E2E_STACK!, 'utf8'));
const shots = path.join(info.work, 'shots');
fs.mkdirSync(shots, { recursive: true });

async function control<T>(method: 'GET' | 'POST', route: string, body?: unknown): Promise<T> {
  const r = await fetch(info.control + route, {
    method,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const answer = await r.json();
  if (!r.ok) throw new Error(`${method} ${route}: ${JSON.stringify(answer)}`);
  return answer as T;
}

const imap = (user: 'alice' | 'bob', mailbox: string) =>
  control<ImapMessage[]>('GET', `/imap?user=${user}&mailbox=${encodeURIComponent(mailbox)}`);
const smtp = () => control<SmtpMessage[]>('GET', '/smtp');
const sha256 = (b: Buffer) => createHash('sha256').update(b).digest('hex');

let shotN = 0;
async function shot(page: Page, name: string) {
  shotN += 1;
  await page.screenshot({
    path: path.join(shots, `${String(shotN).padStart(2, '0')}-${name}.png`),
  });
}

let ctx: BrowserContext;
let page: Page;
const problems: string[] = [];

test.describe.configure({ mode: 'serial' });

test.beforeAll(async ({ browser }) => {
  ctx = await browser.newContext({ timezoneId: 'America/Sao_Paulo' });
  page = await ctx.newPage();
  page.on('pageerror', (e) => problems.push(`[${step}] pageerror: ${e.message}`));
  page.on('console', (m) => {
    if (m.type() === 'error') problems.push(`[${step}] console: ${m.text()}`);
  });
});

let step = '';
test.beforeEach(() => {
  step = test.info().title;
});

test.afterAll(async () => {
  fs.writeFileSync(path.join(shots, 'console.txt'), problems.join('\n') + '\n');
  await ctx?.close();
});

const threadList = () => page.getByRole('region', { name: 'Threads' });
const reader = () => page.getByRole('region', { name: 'Reading pane' });
const row = (subject: string) =>
  threadList()
    .getByRole('listitem')
    .filter({ has: page.getByRole('checkbox', { name: `Select ${subject}` }) });
const status = () => page.getByRole('status', { name: /^Engine:/ });

async function openThread(subject: string) {
  await row(subject).getByRole('link').click();
  await expect(reader().getByRole('heading', { level: 1, name: subject })).toBeVisible();
}

async function goInbox(user: 'alice' | 'bob') {
  const nav = page.getByRole('navigation', { name: 'Mailboxes' });
  // Settings has no mailbox sidebar.
  if (!(await nav.isVisible())) await page.getByRole('link', { name: 'Tern, go to inbox' }).click();
  const account = nav.getByRole('button', { name: new RegExp(`^${info.accounts[user].email}`) });
  if ((await account.getAttribute('aria-expanded')) !== 'true') await account.click();
  // The account's own Inbox link follows its button in the sidebar.
  await nav
    .locator(`a[href^="/${info.accounts[user].id}/"]`)
    .filter({ hasText: /^Inbox/ })
    .click();
  await expect(threadList().getByRole('heading', { name: 'Inbox' })).toBeVisible();
}

function messageFrame(): Promise<Frame> {
  return expect
    .poll(() => page.frames().find((f) => f.url() === 'about:srcdoc') ?? null)
    .not.toBeNull()
    .then(() => page.frames().find((f) => f.url() === 'about:srcdoc')!);
}

test('pairing', async () => {
  // Not paired yet: the app sends the browser to /pair.
  await page.goto(info.origin + '/');
  await expect(page).toHaveURL(/\/pair/);
  await page.goto(info.pair.url);
  await expect(page.getByLabel('Pairing code')).toHaveValue(info.pair.code);
  await shot(page, 'pair');
  await page.getByRole('button', { name: 'Pair' }).click();
  await expect(status()).toHaveAccessibleName('Engine: Connected');
  await expect(row('Quarterly zebra report')).toBeVisible();
  // The fragment is gone from history.
  expect(page.url()).not.toContain(info.pair.code);
  await shot(page, 'inbox');
});

test('inbox and unified lists', async () => {
  await goInbox('alice');
  await expect(row('Garden plans for spring')).toHaveCount(0);
  await page.getByRole('link', { name: /^All inboxes/ }).click();
  await expect(row('Garden plans for spring')).toBeVisible();
  await expect(row('Quarterly zebra report')).toBeVisible();
  await shot(page, 'unified-inbox');
  await goInbox('bob');
  await expect(row('Garden plans for spring')).toBeVisible();
  await expect(row('Quarterly zebra report')).toHaveCount(0);
});

test('opening a thread marks it read on the server', async () => {
  await goInbox('alice');
  await openThread('Re: Message 20');
  await expect(reader().getByRole('article')).toHaveCount(4);
  await shot(page, 'thread');
  await expect
    .poll(async () =>
      (await imap('alice', 'INBOX'))
        .filter((m) => /Message 20$/.test(m.subject))
        .every((m) => m.flags.includes('\\Seen')),
    )
    .toBe(true);
});

test('search with the grammar and dates', async () => {
  const box = page.getByRole('combobox', { name: 'Search mail' });
  const search = async (q: string) => {
    await box.click();
    await box.fill(q);
    await box.press('Enter');
    await expect(page).toHaveURL(/\/search/);
  };
  const results = () => page.getByRole('main');

  await search('from:carol has:attachment');
  await expect(results().getByText('Quarterly zebra report').first()).toBeVisible();
  await expect(results().getByText('Budget planning kickoff')).toHaveCount(0);
  await shot(page, 'search-from-has');

  await search('from:carol before:"30 days ago"');
  await expect(results().getByText('Budget planning kickoff').first()).toBeVisible();
  await expect(results().getByText('Quarterly zebra report')).toHaveCount(0);

  await search('after:yesterday (lighthouse OR garden)');
  // Both accounts in one search.
  await expect(results().getByText('Photos from the lighthouse').first()).toBeVisible();
  await expect(results().getByText('Garden plans for spring').first()).toBeVisible();
  await shot(page, 'search-or-dates');

  await search('from:(');
  await expect(results().getByText('Can’t read that search')).toBeVisible();
  await expect(results().getByText(/from: needs a value/)).toBeVisible();
  await shot(page, 'search-error');
  await box.fill('');
  await box.press('Escape');
});

test('starring and archiving', async () => {
  await goInbox('alice');
  await row('Message 12').getByRole('button', { name: 'Star' }).click();
  await expect
    .poll(async () => (await imap('alice', 'INBOX')).find((m) => m.subject === 'Message 12')?.flags)
    .toContain('\\Flagged');
  await page.getByRole('link', { name: 'Starred' }).first().click();
  await expect(row('Message 12')).toBeVisible();
  await shot(page, 'starred');

  await goInbox('alice');
  await openThread('Message 11');
  await reader().getByRole('button', { name: 'Archive' }).click();
  await expect(row('Message 11')).toHaveCount(0);
  await expect
    .poll(async () => (await imap('alice', 'Archive')).some((m) => m.subject === 'Message 11'))
    .toBe(true);
  expect((await imap('alice', 'INBOX')).some((m) => m.subject === 'Message 11')).toBe(false);
  await shot(page, 'archived');
});

test('attachment download and an inline image', async () => {
  await goInbox('alice');
  await openThread('Quarterly zebra report');
  const link = reader().getByRole('link', { name: /zebra-report\.pdf/ });
  const [download] = await Promise.all([page.waitForEvent('download'), link.click()]);
  expect(download.suggestedFilename()).toBe('zebra-report.pdf');
  const file = await download.path();
  expect(sha256(fs.readFileSync(file))).toBe(info.reportPdfSha256);

  await openThread('Photos from the lighthouse');
  const frame = await messageFrame();
  const img = frame.locator('img[alt="lighthouse"]');
  await expect(img).toHaveAttribute('src', /\/gateway\/v1\/files\/.+\?inline=1$/);
  await expect
    .poll(() => img.evaluate((i) => (i as unknown as { naturalWidth: number }).naturalWidth))
    .toBe(24);
  await shot(page, 'inline-image');
});

test('undo send is an engine setting', async () => {
  await page.getByRole('link', { name: 'Settings' }).click();
  const select = page.locator('#pref-undo-send');
  await expect(select).toHaveText('5 seconds');
  await expect(page.getByText(/Saved in the engine/)).toBeVisible();
  await select.click();
  await page.getByRole('option', { name: '15 seconds' }).click();
  await expect
    .poll(
      async () =>
        (await control<Record<string, unknown>>('GET', '/settings'))['core.sending.undoSend'],
    )
    .toBe(15000);
  // Changed elsewhere (the desktop): the page follows `settings.changed`.
  await control('POST', '/settings', { values: { 'core.sending.undoSend': 0 } });
  await expect(select).toHaveText('Off');
  await shot(page, 'settings-undo-send');
});

test('compose with an attachment and send', async () => {
  await goInbox('alice');
  await page.keyboard.press('c');
  const composer = page
    .getByRole('dialog')
    .or(page.getByRole('region', { name: /New message/ }))
    .first();
  await expect(composer.getByLabel('Subject')).toBeVisible();
  await composer.getByLabel('To', { exact: false }).first().fill('bob@tern.test');
  await composer.getByLabel('Subject').fill('Ferry timetable');
  await composer.getByLabel('Message body').click();
  await page.keyboard.type('The timetable is attached.');
  const payload = Buffer.from('ferry,departs\nnorth,09:15\nsouth,17:40\n');
  await composer.locator('input[type=file]').setInputFiles({
    name: 'ferry.csv',
    mimeType: 'text/csv',
    buffer: payload,
  });
  await expect(
    composer.getByRole('list', { name: 'Attachments' }).getByText('ferry.csv'),
  ).toBeVisible();
  await expect(composer.getByLabel('Uploading')).toHaveCount(0);
  await shot(page, 'composer');
  await composer.getByRole('button', { name: /^Send$/ }).click();

  const sent = await expect
    .poll(async () => (await smtp()).find((m) => m.subject === 'Ferry timetable') ?? null, {
      timeout: 60_000,
    })
    .not.toBeNull()
    .then(async () => (await smtp()).find((m) => m.subject === 'Ferry timetable')!);
  expect(sent.recipients).toEqual(['bob@tern.test']);
  expect(sent.text).toContain('The timetable is attached.');
  const part = sent.parts.find((p) => p.filename === 'ferry.csv');
  expect(part?.sha256).toBe(sha256(payload));

  // Delivered to bob on the fake server, synced, and shown in bob's inbox.
  await goInbox('bob');
  await expect(row('Ferry timetable')).toBeVisible({ timeout: 60_000 });
  await shot(page, 'delivered');
});

test('unsubscribe', async () => {
  await goInbox('alice');
  await openThread('Your weekly digest');
  await reader().getByRole('button', { name: 'Unsubscribe' }).click();
  await expect(page.getByText(/unsubscribe email to leave@news\.example/i)).toBeVisible();
  await expect
    .poll(async () => (await smtp()).some((m) => m.recipients.includes('leave@news.example')), {
      timeout: 60_000,
    })
    .toBe(true);
  await shot(page, 'unsubscribe-mailto');

  await openThread('Autumn offers');
  await reader().getByRole('button', { name: 'Unsubscribe' }).click();
  const toast = page.locator('[data-sonner-toast]').filter({ hasText: /shop\.example/ });
  await expect(toast).toBeVisible();
  await expect(toast.getByRole('button', { name: /Open/ })).toBeVisible();
  await shot(page, 'unsubscribe-web');
});

test('snooze', async () => {
  await goInbox('alice');
  await openThread('Message 10');
  await reader().getByRole('button', { name: 'Snooze' }).click();
  await page.getByRole('menuitem', { name: /^Tomorrow/ }).click();
  await expect(row('Message 10')).toHaveCount(0);
  await expect
    .poll(async () =>
      (await imap('alice', 'Tern/Snoozed')).some((m) => m.subject === 'Message 10'),
    )
    .toBe(true);
  const nav = page.getByRole('navigation', { name: 'Mailboxes' });
  await nav
    .locator(`a[href^="/${info.accounts.alice.id}/"]`)
    .filter({ hasText: /^Snoozed/ })
    .click();
  await expect(row('Message 10')).toBeVisible();
  await shot(page, 'snoozed');
});

test('sign out and pair again', async () => {
  await page.getByRole('button', { name: /^Paired as/ }).click();
  await page.getByRole('menuitem', { name: 'Sign out' }).click();
  await expect(page).toHaveURL(/\/pair/);
  // The page shows /pair as soon as it forgets the session; the unpair POST may still be
  // on its way.
  await expect
    .poll(async () => (await page.request.get(info.origin + '/gateway/v1/session')).status())
    .toBe(401);
  await shot(page, 'signed-out');

  const code = await control<{ code: string; url: string }>('POST', '/pair');
  await page.goto(code.url);
  await page.getByRole('button', { name: 'Pair' }).click();
  await expect(status()).toHaveAccessibleName('Engine: Connected');
  await expect(threadList().getByRole('listitem').first()).toBeVisible();
});

test('engine restart and reconnect', async () => {
  await goInbox('alice');
  await control('POST', '/engine/kill');
  await expect(status()).not.toHaveAccessibleName('Engine: Connected');
  // The gateway's /session says the engine is down once the client checks again.
  await expect(page.getByText("The engine isn't running.").first()).toBeVisible();
  await shot(page, 'engine-down');
  await control('POST', '/engine/start');
  await expect(status()).toHaveAccessibleName('Engine: Connected', { timeout: 60_000 });
  await expect(row('Quarterly zebra report')).toBeVisible();
  // The new engine's deltas reach the list.
  await control('POST', '/deliver?user=alice&subject=After%20the%20restart');
  await page.getByRole('button', { name: 'Check for mail' }).click();
  await expect(row('After the restart')).toBeVisible({ timeout: 60_000 });
  await shot(page, 'reconnected');
});
