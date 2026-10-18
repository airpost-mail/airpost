import type {
  AccountJSON,
  CalendarJSON,
  CategoryJSON,
  CategoryRole,
  ContactJSON,
  ContactRef,
  EventJSON,
  FileJSON,
  LabelJSON,
  MessageJSON,
} from '../../types';
import { PlacementFlags } from '../../types';
import { buildCalendars } from './calendar';
import { ME_GMAIL, ME_IMAP, people } from './people';
import { incoming, scripts, type Place, type ThreadScript, type Who } from './scripts';

export const MOCK_ACCOUNT_IDS = { gmail: 'a1f3c9e2', imap: 'b7204d1a' } as const;
type AccountKey = keyof typeof MOCK_ACCOUNT_IDS;

export interface MockBody {
  body: string;
  plaintext: boolean;
}

export interface Fixtures {
  accounts: AccountJSON[];
  categories: CategoryJSON[];
  /** Without bodies; see `bodies`. */
  messages: MessageJSON[];
  bodies: Map<string, MockBody>;
  /** Message ids whose bodies the engine has "downloaded". */
  loadedBodies: Set<string>;
  contacts: ContactJSON[];
  calendars: CalendarJSON[];
  events: EventJSON[];
}

const accounts: Record<AccountKey, AccountJSON> = {
  gmail: {
    id: MOCK_ACCOUNT_IDS.gmail,
    aid: MOCK_ACCOUNT_IDS.gmail,
    v: 1,
    __cls: 'Account',
    name: ME_GMAIL.name!,
    provider: 'gmail',
    emailAddress: ME_GMAIL.email,
    label: 'Personal',
    settings: {
      imap_host: 'imap.gmail.com',
      imap_port: 993,
      imap_security: 'SSL / TLS',
      smtp_host: 'smtp.gmail.com',
      smtp_port: 465,
      smtp_security: 'SSL / TLS',
    },
    autoaddress: { type: 'bcc', value: '' },
    aliases: [],
    defaultAlias: null,
    syncState: 'ok',
    syncError: null,
    color: '#3b6fe0',
  },
  imap: {
    id: MOCK_ACCOUNT_IDS.imap,
    aid: MOCK_ACCOUNT_IDS.imap,
    v: 1,
    __cls: 'Account',
    name: ME_IMAP.name!,
    provider: 'imap',
    emailAddress: ME_IMAP.email,
    label: 'Studio',
    settings: {
      imap_host: 'mail.okafor.studio',
      imap_port: 993,
      imap_username: ME_IMAP.email,
      imap_security: 'SSL / TLS',
      smtp_host: 'mail.okafor.studio',
      smtp_port: 587,
      smtp_username: ME_IMAP.email,
      smtp_security: 'STARTTLS',
    },
    autoaddress: { type: 'bcc', value: '' },
    aliases: ['hello@okafor.studio'],
    defaultAlias: null,
    syncState: 'ok',
    syncError: null,
    color: '#d97a2b',
  },
};

function cat(
  cls: 'Folder' | 'Label',
  aid: string,
  id: string,
  path: string,
  role: CategoryRole,
): CategoryJSON {
  return { id, aid, v: 1, __cls: cls, path, role, localStatus: {} };
}

const G = MOCK_ACCOUNT_IDS.gmail;
const I = MOCK_ACCOUNT_IDS.imap;

function buildCategories(): CategoryJSON[] {
  return [
    cat('Folder', G, 'g-all', '[Gmail]/All Mail', 'all'),
    cat('Folder', G, 'g-spam', '[Gmail]/Spam', 'spam'),
    cat('Folder', G, 'g-trash', '[Gmail]/Trash', 'trash'),
    cat('Label', G, 'g-inbox', 'INBOX', 'inbox'),
    cat('Label', G, 'g-sent', '[Gmail]/Sent Mail', 'sent'),
    cat('Label', G, 'g-drafts', '[Gmail]/Drafts', 'drafts'),
    cat('Label', G, 'g-important', '[Gmail]/Important', 'important'),
    cat('Label', G, 'g-family', 'Family', ''),
    cat('Label', G, 'g-travel', 'Travel', ''),
    cat('Label', G, 'g-receipts', 'Receipts', ''),
    cat('Folder', I, 'i-inbox', 'INBOX', 'inbox'),
    cat('Folder', I, 'i-sent', 'Sent', 'sent'),
    cat('Folder', I, 'i-drafts', 'Drafts', 'drafts'),
    cat('Folder', I, 'i-archive', 'Archive', 'archive'),
    cat('Folder', I, 'i-spam', 'Junk', 'spam'),
    cat('Folder', I, 'i-trash', 'Trash', 'trash'),
    cat('Folder', I, 'i-clients', 'INBOX/Clients', ''),
    cat('Folder', I, 'i-northwind', 'INBOX/Clients/Northwind', ''),
    cat('Folder', I, 'i-invoices', 'Invoices', ''),
    cat('Folder', I, 'i-cafe', 'Caf&AOk-', ''),
  ];
}

const GMAIL_ROLE_XGM: Partial<Record<CategoryRole, string>> = {
  inbox: '\\Inbox',
  sent: '\\Sent',
  drafts: '\\Draft',
  important: '\\Important',
};

/** The X-GM-LABELS value Gmail reports for a label. */
export function xgmName(label: Pick<LabelJSON, 'path' | 'role'>): string {
  return GMAIL_ROLE_XGM[label.role] ?? label.path;
}

export function htmlToText(html: string): string {
  return html
    .replace(/<(style|script|head)[\s\S]*?<\/\1>/gi, ' ')
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<[^>]+>/g, ' ')
    .replace(/&nbsp;/g, ' ')
    .replace(/&amp;/g, '&')
    .replace(/&rarr;/g, '→')
    .replace(/&[a-z]+;/g, ' ')
    .replace(/[ \t]+/g, ' ')
    .trim();
}

export function snippetOf(body: string, plaintext: boolean): string {
  const text = plaintext ? body : htmlToText(body);
  return text.replace(/\s+/g, ' ').trim().slice(0, 160);
}

export class FixtureBuilder {
  readonly messages: MessageJSON[] = [];
  readonly bodies = new Map<string, MockBody>();
  readonly loadedBodies = new Set<string>();
  private seq: number;
  private readonly categories: CategoryJSON[];

  constructor(categories: CategoryJSON[], startSeq = 0) {
    this.categories = categories;
    this.seq = startSeq;
  }

  private byPath(aid: string, path: string): CategoryJSON {
    const c = this.categories.find((x) => x.aid === aid && x.path === path);
    if (!c) throw new Error(`fixture: no category ${path}`);
    return c;
  }

  private byRole(aid: string, role: CategoryRole): CategoryJSON {
    const c = this.categories.find((x) => x.aid === aid && x.role === role);
    if (!c) throw new Error(`fixture: no role ${role}`);
    return c;
  }

  /** Adds a scripted thread whose last message is at `now - hoursAgo`. Returns the thread id. */
  add(script: ThreadScript, now: number): string {
    const n = ++this.seq;
    const key = script.account;
    const aid = MOCK_ACCOUNT_IDS[key];
    const me = key === 'gmail' ? ME_GMAIL : ME_IMAP;
    const who = (w: Who): ContactRef => (w === 'me' ? me : people[w]);
    const threadId = `t-${key[0]}-${String(n).padStart(3, '0')}`;
    const gThrId = key === 'gmail' ? String(1790000000000000000n + BigInt(n * 7919)) : undefined;

    const msgs = script.messages;
    const dates: number[] = new Array(msgs.length);
    let t = now - Math.round(script.hoursAgo * 3600);
    for (let i = msgs.length - 1; i >= 0; i--) {
      dates[i] = t;
      t -= Math.round((msgs[i].gap ?? 3) * 3600);
    }

    const unreadFrom = msgs.length - (script.unread ?? 0);
    let prevHeader: string | null = null;
    const counterpart = msgs.find((m) => m.from !== 'me')?.from ?? 'jonas';

    msgs.forEach((m, i) => {
      const id = `m-${key[0]}-${String(n).padStart(3, '0')}-${i}`;
      const fromMe = m.from === 'me';
      const draft = script.place === 'drafts' && fromMe;
      const unread = !fromMe && i >= unreadFrom;
      const starred = !!script.starred && i === msgs.length - 1;
      const to = (m.to ?? (fromMe ? [counterpart] : ['me'])).map(who);
      const plaintext = m.html === undefined;
      const body = m.html ?? m.text ?? '';
      const bits =
        (unread ? PlacementFlags.Unread : 0) |
        (starred ? PlacementFlags.Starred : 0) |
        (draft ? PlacementFlags.Draft : 0);

      const { folders, labels } = this.placement(script, aid, fromMe, draft, bits);
      const hMsgId = `${id}@mock.tern`;
      const files: FileJSON[] = (m.files ?? []).map((f, fi) => ({
        id: `f-${id}-${fi}`,
        aid,
        v: 1,
        __cls: 'File',
        messageId: id,
        partId: String(fi + 2),
        contentId: f.inline ? f.filename.replace(/\.\w+$/, '') + '@tidepool' : null,
        contentType: f.contentType,
        filename: f.filename,
        size: f.size,
      }));

      const message: MessageJSON = {
        id,
        aid,
        v: 1,
        __cls: 'Message',
        subject: i === 0 ? script.subject : script.subject ? `Re: ${script.subject}` : '',
        snippet: snippetOf(body, plaintext),
        date: dates[i],
        threadId,
        hMsgId,
        rthMsgId: prevHeader,
        gMsgId: gThrId ? String(BigInt(gThrId) + BigInt(i)) : undefined,
        unread,
        starred,
        draft,
        plaintext,
        folders,
        labels,
        from: [who(m.from)],
        to,
        cc: (m.cc ?? []).map(who),
        bcc: [],
        replyTo: [],
        files,
        events: [],
        extraHeaders: {},
      };
      if (m.from === 'weekly' || m.from === 'rustnews') {
        message.hListUnsub = '<https://unsubscribe.example/u/123>';
        message.hListUnsubPost = 'List-Unsubscribe=One-Click';
      }
      this.messages.push(message);
      this.bodies.set(id, { body, plaintext });
      if (!script.noBody) this.loadedBodies.add(id);
      prevHeader = hMsgId;
    });
    return threadId;
  }

  private placement(
    script: ThreadScript,
    aid: string,
    fromMe: boolean,
    draft: boolean,
    bits: number,
  ): { folders: Record<string, number>; labels: string[] } {
    const place: Place = script.place;
    if (script.account === 'imap') {
      let folder: CategoryJSON;
      if (draft) folder = this.byRole(aid, 'drafts');
      else if (fromMe && place !== 'trash' && place !== 'spam') folder = this.byRole(aid, 'sent');
      else if (script.folder) folder = this.byPath(aid, script.folder);
      else if (place === 'sent') folder = this.byRole(aid, 'archive');
      else folder = this.byRole(aid, place);
      return { folders: { [folder.id]: bits }, labels: [] };
    }

    if (place === 'spam' || place === 'trash') {
      return { folders: { [this.byRole(aid, place).id]: bits }, labels: [] };
    }
    const labels: string[] = [];
    if (draft) labels.push('\\Draft');
    else if (fromMe) labels.push('\\Sent');
    else if (place === 'inbox') labels.push('\\Inbox');
    if (script.important && !fromMe) labels.push('\\Important');
    labels.push(...(script.labels ?? []));
    return { folders: { [this.byRole(aid, 'all').id]: bits }, labels };
  }
}

function buildContacts(messages: MessageJSON[]): ContactJSON[] {
  const out = new Map<string, ContactJSON>();
  for (const m of messages) {
    const own = Object.values(accounts).find((a) => a.id === m.aid)!.emailAddress;
    for (const c of [...m.from, ...m.to, ...m.cc]) {
      if (c.email === own) continue;
      const id = `c-${m.aid}-${c.email}`;
      const existing = out.get(id);
      if (existing) {
        existing.refs += 1;
        continue;
      }
      out.set(id, {
        id,
        aid: m.aid,
        v: 1,
        __cls: 'Contact',
        name: c.name ?? '',
        email: c.email,
        s: 'mail',
        refs: 1,
        gis: [],
        info: {},
      });
    }
  }
  return [...out.values()];
}

/** Deterministic for a given `now` (unix seconds). */
export function buildFixtures(now: number): Fixtures {
  const categories = buildCategories();
  const builder = new FixtureBuilder(categories);
  for (const s of scripts) builder.add(s, now);
  return {
    accounts: Object.values(accounts),
    categories,
    messages: builder.messages,
    bodies: builder.bodies,
    loadedBodies: builder.loadedBodies,
    contacts: buildContacts(builder.messages),
    ...buildCalendars(now, MOCK_ACCOUNT_IDS),
  };
}

export { incoming };
