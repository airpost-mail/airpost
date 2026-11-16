import { newsletter, notification, personal, receipt, suspicious } from './html';
import type { PersonKey } from './people';

export type Place = 'inbox' | 'sent' | 'drafts' | 'archive' | 'spam' | 'trash';
export type Who = PersonKey | 'me';

export interface FileScript {
  filename: string;
  contentType: string;
  size: number;
  inline?: boolean;
}

export interface MsgScript {
  from: Who;
  to?: Who[];
  cc?: Who[];
  text?: string;
  html?: string;
  files?: FileScript[];
  /** Hours between the previous message and this one. */
  gap?: number;
}

export interface ThreadScript {
  account: 'gmail' | 'imap';
  place: Place;
  /** IMAP user folder path, used instead of the place's folder. */
  folder?: string;
  /** Gmail user labels. */
  labels?: string[];
  important?: boolean;
  subject: string;
  /** Age of the last message in hours. */
  hoursAgo: number;
  /** Unread messages counted from the end of the thread. */
  unread?: number;
  starred?: boolean;
  /** Bodies not downloaded yet; the client has to call bodies.fetch. */
  noBody?: boolean;
  messages: MsgScript[];
}

const pdf = (filename: string, size = 184_000): FileScript => ({
  filename,
  contentType: 'application/pdf',
  size,
});
const png = (filename: string, size = 412_000, inline = false): FileScript => ({
  filename,
  contentType: 'image/png',
  size,
  inline,
});

export const scripts: ThreadScript[] = [
  // ---------------------------------------------------------------- IMAP: work (okafor.studio)
  {
    account: 'imap',
    place: 'inbox',
    subject: 'Northwind rebrand — round 2 feedback',
    hoursAgo: 0.4,
    unread: 2,
    starred: true,
    messages: [
      {
        from: 'jonas',
        cc: ['priya'],
        text: 'Hi Maya,\n\nWe went through the second round with the board yesterday. Overall very positive. The wordmark is a clear winner, the monogram less so.\n\nTwo asks:\n1. Can we see the wordmark at 16px for the app icon?\n2. The teal feels cold next to the photography. Is there a warmer variant?\n\nJonas',
      },
      {
        from: 'me',
        to: ['jonas'],
        cc: ['priya'],
        gap: 5,
        text: 'Thanks Jonas, glad the wordmark landed.\n\n16px tests are easy, I will send them tomorrow. For the teal I have two warmer options in mind, one shifted toward green, one toward sand. I will mock both against your photo library.\n\nMaya',
      },
      {
        from: 'priya',
        to: ['me'],
        cc: ['jonas'],
        gap: 2,
        text: 'Quick one from me: marketing wants to keep the teal for the trade show booth since the panels are already printed. Can the warmer variant be secondary rather than a replacement?\n\nPriya',
      },
      {
        from: 'me',
        to: ['priya'],
        cc: ['jonas'],
        gap: 1,
        text: 'That works well actually. Teal stays primary, the warm tone becomes the accent for editorial and social. I will set it up that way.',
      },
      {
        from: 'jonas',
        to: ['me'],
        cc: ['priya'],
        gap: 20,
        text: 'Perfect. Looking forward to the icon tests.',
      },
      {
        from: 'me',
        to: ['jonas', 'priya'],
        gap: 16,
        html: personal([
          'Here are the 16px and 32px tests, plus the two warm accents side by side.',
          'My vote is <b>Sand 2</b>: it holds up next to the teal and survives print.',
        ]),
        files: [
          png('northwind-icon-16-32.png', 96_000),
          pdf('northwind-accent-options.pdf', 2_340_000),
        ],
      },
      {
        from: 'priya',
        to: ['me'],
        cc: ['jonas'],
        gap: 3,
        text: 'Sand 2 for me too. The 16px icon is a bit muddy on dark backgrounds though, the counter of the "n" closes up.',
      },
      {
        from: 'jonas',
        to: ['me'],
        cc: ['priya'],
        gap: 1,
        text: 'Agree on Sand 2. On the icon: could we try a slightly bolder cut only for the small sizes? I think that is what the foundry calls an optical size.',
      },
      {
        from: 'priya',
        to: ['me'],
        cc: ['jonas'],
        gap: 0.2,
        text: 'Also, the board meets again on the 24th. If we can have the final package by the 21st that would be ideal.',
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: 'Contract amendment — Harbour Labs retainer',
    hoursAgo: 3,
    unread: 1,
    important: true,
    messages: [
      {
        from: 'omar',
        cc: ['ines'],
        text: 'Hi Maya,\n\nAs discussed, we would like to extend the retainer to cover the product illustration work through Q2. Our legal team will send an amendment.\n\nBest,\nOmar',
      },
      {
        from: 'me',
        to: ['omar'],
        cc: ['ines'],
        gap: 4,
        text: 'Great news, thank you Omar. I will loop in my lawyer, Felix, so he can review the amendment directly.',
      },
      {
        from: 'felix',
        to: ['me'],
        gap: 30,
        text: "Maya,\n\nI read the draft amendment. Two points:\n\n- Clause 4.2 assigns IP on delivery, not on payment. We should switch that.\n- The termination notice is 7 days, which is short for a retainer. I suggest 30.\n\nOtherwise it's fine.\n\nFelix",
        files: [pdf('Harbour-Labs-Amendment-v1-FB-comments.pdf', 220_000)],
      },
      {
        from: 'me',
        to: ['omar'],
        cc: ['felix', 'ines'],
        gap: 2,
        text: 'Omar, Felix has two small changes (IP on payment, 30-day notice). His marked-up copy is attached. Could your legal team take a look?',
        files: [pdf('Harbour-Labs-Amendment-v1-FB-comments.pdf', 220_000)],
      },
      {
        from: 'ines',
        to: ['me'],
        cc: ['omar', 'felix'],
        gap: 22,
        text: 'Both changes accepted. Revised version attached for signature.',
        files: [pdf('Harbour-Labs-Amendment-v2.pdf', 205_000)],
      },
      {
        from: 'felix',
        to: ['me'],
        gap: 1,
        text: 'v2 reflects both changes. Good to sign.',
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: 'Illustration brief: onboarding screens',
    hoursAgo: 5,
    unread: 1,
    messages: [
      {
        from: 'ines',
        text: 'Hi Maya,\n\nAttached is the brief for the five onboarding illustrations. Style reference is the set you did for the landing page. Deadline is flexible, end of month would be great.\n\nInês',
        files: [pdf('onboarding-brief.pdf', 640_000), png('reference-landing.png', 1_200_000)],
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: '[Linear] AIRPOST-214 Thread list jumps when a delta arrives',
    hoursAgo: 7,
    unread: 1,
    messages: [
      {
        from: 'linear',
        html: notification({
          service: 'Linear',
          headline: 'Sam Whitfield assigned AIRPOST-214 to you',
          body: 'When a Thread delta arrives for a row above the viewport, the list scrolls by one row. Repro: open Inbox, scroll down, mark a thread above as read from another device.',
          cta: 'Open issue',
          href: 'https://linear.example/airpost/issue/AIRPOST-214',
          accent: '#5e6ad2',
        }),
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: 'Studio visit next Thursday?',
    hoursAgo: 9,
    messages: [
      {
        from: 'theo',
        text: "Salut Maya,\n\nI'll be in town next week for the print fair. Could I drop by the studio on Thursday afternoon? I'd love to see the risograph prints in person.\n\nThéo",
      },
      {
        from: 'me',
        to: ['theo'],
        gap: 2,
        text: 'Of course! Thursday after 3 works. I will put the kettle on.',
      },
      {
        from: 'theo',
        to: ['me'],
        gap: 1,
        text: 'Parfait. À jeudi!',
      },
    ],
  },
  {
    account: 'imap',
    place: 'archive',
    subject: 'Invoice 2026-031 paid',
    hoursAgo: 14,
    folder: 'Invoices',
    messages: [
      {
        from: 'stripe',
        html: receipt({
          vendor: 'Stripe on behalf of Okafor Studio',
          orderId: '2026-031',
          lines: [
            { item: 'Northwind — brand identity, milestone 2', amount: '€4,800.00' },
            { item: 'Expenses (printing proofs)', amount: '€212.40' },
          ],
          total: '€5,012.40',
        }),
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: 'Comments on "Harbour onboarding v3"',
    hoursAgo: 20,
    unread: 1,
    messages: [
      {
        from: 'figma',
        html: notification({
          service: 'Figma',
          headline: 'Omar Haddad and 2 others commented',
          body: '<b>Omar:</b> Can the character on screen 3 hold the phone in the other hand?<br><b>Inês:</b> Love the palette.<br><b>Sam:</b> Screen 5 copy is placeholder, ignore.',
          cta: 'View comments',
          href: 'https://figma.example/file/abc',
          accent: '#0d99ff',
        }),
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: 'Re: Guest lecture on brand systems',
    hoursAgo: 26,
    messages: [
      {
        from: 'me',
        to: ['noor'],
        text: 'Hi Noor,\n\nI would be happy to give the lecture. Would 60 minutes plus questions suit?\n\nMaya',
      },
      {
        from: 'noor',
        to: ['me'],
        gap: 19,
        text: 'Wonderful! 60 minutes is perfect. The students will have read your essay on modular identities beforehand. I will send the room details and the honorarium form.\n\nNoor',
        files: [pdf('honorarium-form.pdf', 98_000)],
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: 'Print proofs ready for pickup',
    hoursAgo: 30,
    messages: [
      {
        from: 'hana',
        text: 'Hi Maya, the riso proofs are ready. 2 colours, 120 copies, the pink came out a little hot. Come by when you can, we are open until 6.\n\nHana',
        files: [png('proof-photo.jpg', 2_800_000)],
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: 'Quarterly VAT reminder',
    hoursAgo: 52,
    messages: [
      {
        from: 'bank',
        html: notification({
          service: 'Harbour Bank Business',
          headline: 'Your quarterly VAT payment is due in 14 days',
          body: 'Based on your invoices, the estimated amount is <b>€3,184.20</b>. You can schedule the payment now.',
          cta: 'Schedule payment',
          href: 'https://harbourbank.example/vat',
          accent: '#0b6e4f',
        }),
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: 'Workshop: type design for small sizes',
    hoursAgo: 70,
    noBody: true,
    messages: [
      {
        from: 'lena',
        text: 'Hi Maya,\n\nWe are running a two-day workshop in March on optical sizes and hinting. Given your Northwind project you might find it useful. Early bird ends Friday.\n\nLena',
      },
    ],
  },
  {
    account: 'imap',
    place: 'sent',
    subject: 'Proposal: Tidepool annual report',
    hoursAgo: 40,
    messages: [
      {
        from: 'me',
        to: ['rosa'],
        text: 'Hi Rosa,\n\nAttached is the proposal for the annual report: 32 pages, two illustration spreads, print and web. Happy to walk you through it on a call.\n\nMaya',
        files: [pdf('Tidepool-annual-report-proposal.pdf', 1_100_000)],
      },
    ],
  },
  {
    account: 'imap',
    place: 'sent',
    subject: 'Re: Invoice 2026-029',
    hoursAgo: 120,
    messages: [
      {
        from: 'omar',
        text: 'Hi Maya, could you resend invoice 029 with our new VAT number? Thanks!',
      },
      {
        from: 'me',
        to: ['omar'],
        gap: 3,
        text: 'Sure, here it is with the updated VAT number.',
        files: [pdf('Invoice-2026-029.pdf', 76_000)],
      },
    ],
  },
  {
    account: 'imap',
    place: 'sent',
    subject: 'Availability in April',
    hoursAgo: 200,
    messages: [
      {
        from: 'me',
        to: ['sam'],
        text: 'Hi Sam, quick heads up: I am away 14–21 April. Anything urgent for Harbour before then, let me know this week.',
      },
    ],
  },
  {
    account: 'imap',
    place: 'drafts',
    subject: 'Tidepool kickoff agenda',
    hoursAgo: 1,
    messages: [
      {
        from: 'me',
        to: ['rosa'],
        text: 'Hi Rosa,\n\nFor Monday I suggest:\n1. Goals for the report\n2. Content inventory\n3. ',
      },
    ],
  },
  {
    account: 'imap',
    place: 'archive',
    folder: 'INBOX/Clients/Northwind',
    subject: 'Northwind rebrand — kickoff notes',
    hoursAgo: 600,
    noBody: true,
    messages: [
      {
        from: 'jonas',
        cc: ['priya'],
        text: 'Notes from today attached. Thanks for a great session.',
        files: [pdf('kickoff-notes.pdf')],
      },
      {
        from: 'me',
        to: ['jonas'],
        gap: 3,
        text: 'Thanks Jonas, all clear. First concepts in two weeks.',
      },
    ],
  },
  {
    account: 'imap',
    place: 'archive',
    subject: 'Your domain okafor.studio renews soon',
    hoursAgo: 900,
    messages: [
      {
        from: 'stripe',
        html: notification({
          service: 'Domains',
          headline: 'okafor.studio renews on 2 March',
          body: 'Your card ending 4242 will be charged €24.00.',
          cta: 'Manage domain',
          href: 'https://domains.example/okafor.studio',
          accent: '#635bff',
        }),
      },
    ],
  },
  {
    account: 'imap',
    place: 'archive',
    subject: 'Re: Font licensing for Harbour',
    hoursAgo: 1100,
    messages: [
      { from: 'sam', text: 'Do we need a separate web licence for the display face?' },
      {
        from: 'me',
        to: ['sam'],
        gap: 2,
        text: 'Yes, desktop and web are licensed separately. I will get a quote for 50k monthly views.',
      },
      { from: 'sam', to: ['me'], gap: 5, text: 'Great, thanks.' },
    ],
  },
  {
    account: 'imap',
    place: 'archive',
    folder: 'INBOX/Clients',
    subject: 'Tidepool: content inventory',
    hoursAgo: 300,
    messages: [
      {
        from: 'rosa',
        text: 'Spreadsheet with all the sections and owners is attached.',
        files: [
          {
            filename: 'content-inventory.xlsx',
            contentType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
            size: 48_000,
          },
        ],
      },
    ],
  },
  {
    account: 'imap',
    place: 'spam',
    subject: 'URGENT: claim your crypto reward',
    hoursAgo: 11,
    unread: 1,
    messages: [{ from: 'spammer1', html: suspicious() }],
  },
  {
    account: 'imap',
    place: 'spam',
    subject: 'Lowest prices — 90% off today only',
    hoursAgo: 60,
    unread: 1,
    messages: [{ from: 'spammer2', text: 'Best deals online. Click now. No prescription needed.' }],
  },
  {
    account: 'imap',
    place: 'trash',
    subject: 'Webinar: 10 branding mistakes',
    hoursAgo: 80,
    messages: [
      {
        from: 'weekly',
        html: newsletter({
          title: 'Branding Live',
          issue: 'Webinar invite',
          brandColor: '#d1495b',
          items: [
            {
              heading: 'Mistake #1: no system',
              text: 'Join us Thursday for an hour on brand systems.',
            },
          ],
        }),
      },
    ],
  },
  {
    account: 'imap',
    place: 'trash',
    subject: 'Re: lunch?',
    hoursAgo: 400,
    messages: [
      { from: 'sam', text: 'Lunch Friday?' },
      { from: 'me', to: ['sam'], gap: 1, text: 'Can not this week, next Friday?' },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: '[Linear] Weekly summary for Harbour Labs',
    hoursAgo: 96,
    messages: [
      {
        from: 'linear',
        html: notification({
          service: 'Linear',
          headline: '12 issues completed this week',
          body: 'Top contributors: Sam Whitfield (5), Inês Carvalho (4), Maya Okafor (3). 3 issues are overdue.',
          cta: 'View summary',
          href: 'https://linear.example/harbour/summary',
          accent: '#5e6ad2',
        }),
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: 'Moodboard for the annual report',
    hoursAgo: 110,
    starred: true,
    messages: [
      {
        from: 'rosa',
        html: personal([
          'Some images that capture the tone we are after: tidal, a bit hand-made, not corporate.',
          'The inline one below is my favourite.',
          '<img src="cid:moodboard-1@tidepool" alt="Moodboard" width="420">',
        ]),
        files: [png('moodboard-1.png', 820_000, true), png('moodboard-2.png', 760_000)],
      },
    ],
  },

  // ---------------------------------------------------------------- Gmail: personal
  {
    account: 'gmail',
    place: 'inbox',
    labels: ['Family'],
    important: true,
    subject: "Dad's 70th — planning",
    hoursAgo: 1.2,
    unread: 3,
    messages: [
      {
        from: 'amara',
        to: ['me'],
        text: "Maya! Dad turns 70 in June. Should we do something big? I'm thinking the lake house, everyone together.",
      },
      {
        from: 'me',
        to: ['amara'],
        gap: 2,
        text: 'Yes! The lake house is perfect. Weekend of the 14th?',
      },
      {
        from: 'amara',
        to: ['me'],
        gap: 1,
        text: "14th works for me. I'll ask Uncle Emeka about the boat.",
      },
      {
        from: 'me',
        to: ['amara'],
        gap: 26,
        text: 'I can do the invitations. Something illustrated, obviously.',
      },
      {
        from: 'amara',
        to: ['me'],
        gap: 3,
        text: 'Obviously 😄 Should we tell Dad or keep it a surprise?',
      },
      {
        from: 'dad',
        to: ['me', 'amara'],
        gap: 40,
        text: 'Girls, your mother told me you are planning something. I want no fuss. A cake is enough.\n\nLove, Dad',
      },
      { from: 'amara', to: ['me'], gap: 0.5, text: 'Well, so much for the surprise.' },
      {
        from: 'amara',
        to: ['me'],
        gap: 0.3,
        text: 'We are still doing the lake house though. He will love it.',
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    labels: ['Travel'],
    subject: 'Lisbon in May?',
    hoursAgo: 4,
    unread: 1,
    starred: true,
    messages: [
      {
        from: 'marcus',
        to: ['me', 'kenji'],
        text: 'Long weekend in Lisbon, end of May? Flights are cheap right now.',
      },
      { from: 'kenji', to: ['marcus', 'me'], gap: 3, text: "I'm in. 23–26?" },
      {
        from: 'me',
        to: ['marcus', 'kenji'],
        gap: 1,
        text: 'In! I can look for an apartment in Alfama.',
      },
      {
        from: 'marcus',
        to: ['me', 'kenji'],
        gap: 8,
        text: 'Found flights for €118 return. Booking tonight unless someone objects.',
      },
      {
        from: 'me',
        to: ['marcus', 'kenji'],
        gap: 12,
        html: personal([
          'Two options for apartments, both near the tram 28 line:',
          '<a href="https://stays.example/alfama-1">Alfama, terrace</a><br><a href="https://stays.example/graca-2">Graça, 3 bedrooms</a>',
        ]),
      },
      { from: 'kenji', to: ['marcus', 'me'], gap: 2, text: 'The terrace one. Sunset beers.' },
      {
        from: 'marcus',
        to: ['me', 'kenji'],
        gap: 1,
        text: 'Terrace it is. Maya, can you book? I will send you my share.',
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    labels: ['Travel'],
    subject: 'Your booking FJ 2231 — Copenhagen to Lisbon',
    hoursAgo: 6,
    messages: [
      {
        from: 'airline',
        html: receipt({
          vendor: 'Fjord Air',
          orderId: 'FJ-8K2Q9',
          lines: [
            { item: 'CPH → LIS, 23 May, 07:40', amount: '€59.00' },
            { item: 'LIS → CPH, 26 May, 19:15', amount: '€59.00' },
            { item: 'Seat 14A', amount: '€0.00' },
          ],
          total: '€118.00',
        }),
        files: [pdf('boarding-pass-FJ2231.pdf', 140_000)],
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    subject: 'Book club: next pick',
    hoursAgo: 10,
    unread: 1,
    messages: [
      {
        from: 'lena',
        to: ['me', 'rosa', 'hana'],
        text: 'Votes so far: "The Overstory" 2, "Piranesi" 3. Last chance to vote before Sunday!',
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    labels: ['Receipts'],
    subject: 'Your Lantern Books order has shipped',
    hoursAgo: 18,
    messages: [
      {
        from: 'bookshop',
        html: receipt({
          vendor: 'Lantern Books',
          orderId: 'LB-55120',
          lines: [
            { item: 'Piranesi — Susanna Clarke', amount: '€14.90' },
            { item: 'Grid Systems in Graphic Design — J. Müller-Brockmann', amount: '€42.00' },
            { item: 'Shipping', amount: '€3.50' },
          ],
          total: '€60.40',
        }),
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    subject: 'The Interface Weekly #212: Calm software',
    hoursAgo: 22,
    unread: 1,
    messages: [
      {
        from: 'weekly',
        html: newsletter({
          title: 'The Interface Weekly',
          issue: 'Issue #212',
          brandColor: '#1f3b73',
          items: [
            {
              heading: 'Calm software',
              text: 'Why the best tools disappear: a look at interfaces that respect attention.',
            },
            {
              heading: 'Keyboard-first, not keyboard-only',
              text: 'How to design shortcuts that newcomers discover and experts love.',
            },
            {
              heading: 'Density is a feature',
              text: 'Information-dense layouts, done carefully, beat whitespace for daily tools.',
            },
          ],
        }),
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    subject: 'This Week in Rust 640',
    hoursAgo: 34,
    messages: [
      {
        from: 'rustnews',
        html: newsletter({
          title: 'This Week in Rust',
          issue: 'Issue 640',
          brandColor: '#a72145',
          items: [
            {
              heading: 'Async closures stabilised',
              text: 'A long-awaited feature lands in stable.',
            },
            {
              heading: 'Crate of the week',
              text: 'A small, fast JSON-RPC implementation with no dependencies.',
            },
          ],
        }),
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    subject: '[airpost/airpost] Socket transport for the engine (#482)',
    hoursAgo: 38,
    unread: 2,
    messages: [
      {
        from: 'github',
        text: 'kenji-w opened a pull request: Socket transport for the engine\n\nAdds `--socket <path>` to sync mode, newline-delimited JSON-RPC.\n\n—\nReply to this email directly or view it on GitHub.',
      },
      {
        from: 'github',
        gap: 3,
        text: '@samw commented on this pull request.\n\n> The peer uid check should happen before hello.\n\nAgreed, moved it.\n\n—\nReply to this email directly or view it on GitHub.',
      },
      {
        from: 'github',
        gap: 4,
        text: 'kenji-w pushed 2 commits.\n\n- Move peer check before hello\n- Add slow-subscriber test\n\n—\nReply to this email directly or view it on GitHub.',
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    subject: 'Dentist appointment reminder',
    hoursAgo: 46,
    messages: [
      {
        from: 'dentist',
        text: 'Dear Maya Okafor,\n\nThis is a reminder of your appointment on Tuesday at 09:30 with Dr. Weber.\n\nTo reschedule, reply to this email or call us.\n\nLindenhof Dental',
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    subject: 'Building maintenance on the 12th',
    hoursAgo: 75,
    messages: [
      {
        from: 'landlord',
        text: 'Dear tenants,\n\nThe water will be shut off on the 12th between 9:00 and 13:00 for pipe maintenance. We apologise for the inconvenience.\n\nKeystone Property Management',
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    labels: ['Family'],
    subject: 'Photos from the weekend',
    hoursAgo: 90,
    starred: true,
    messages: [
      {
        from: 'amara',
        to: ['me', 'dad'],
        text: 'A few from Sunday. The one of Dad with the fish is going on the fridge.',
        files: [
          png('IMG_4021.jpg', 3_100_000),
          png('IMG_4022.jpg', 2_900_000),
          png('IMG_4030.jpg', 3_400_000),
        ],
      },
      { from: 'dad', to: ['amara', 'me'], gap: 6, text: 'That fish was at least twice that size.' },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    subject: 'Coffee next week?',
    hoursAgo: 130,
    messages: [
      {
        from: 'kenji',
        text: 'Back from Osaka on Sunday. Coffee next week? I brought you something.',
      },
      {
        from: 'me',
        to: ['kenji'],
        gap: 5,
        text: 'Yes please! Wednesday morning at the usual place?',
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    labels: ['Receipts'],
    subject: 'Your train tickets: Copenhagen → Stockholm',
    hoursAgo: 160,
    noBody: true,
    messages: [
      {
        from: 'rail',
        html: receipt({
          vendor: 'Nordic Rail',
          orderId: 'NR-77310',
          lines: [{ item: 'CPH → STO, 2nd class, seat 54', amount: 'SEK 649' }],
          total: 'SEK 649',
        }),
        files: [pdf('ticket-NR-77310.pdf', 88_000)],
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    subject: 'Monthly statement available',
    hoursAgo: 220,
    messages: [
      {
        from: 'bank',
        html: notification({
          service: 'Harbour Bank',
          headline: 'Your statement for February is ready',
          body: 'Log in to view and download your statement.',
          cta: 'View statement',
          href: 'https://harbourbank.example/statements',
          accent: '#0b6e4f',
        }),
      },
    ],
  },
  {
    account: 'gmail',
    place: 'sent',
    subject: 'Recipe you asked for',
    hoursAgo: 28,
    messages: [
      {
        from: 'me',
        to: ['hana'],
        text: "Here's the jollof recipe. The trick is to let the bottom catch a little at the end.\n\n- 3 cups long grain rice\n- 6 tomatoes, 2 red peppers, 1 scotch bonnet\n- 2 onions\n- stock, thyme, bay, curry powder",
      },
    ],
  },
  {
    account: 'gmail',
    place: 'sent',
    subject: 'Re: Apartment in Alfama',
    hoursAgo: 3.5,
    messages: [
      {
        from: 'me',
        to: ['marcus', 'kenji'],
        text: 'Booked! €96 each for three nights. Confirmation attached.',
        files: [pdf('booking-confirmation.pdf', 110_000)],
      },
    ],
  },
  {
    account: 'gmail',
    place: 'sent',
    subject: 'Thank you',
    hoursAgo: 250,
    messages: [
      {
        from: 'me',
        to: ['noor'],
        text: 'Thank you for having me, the students asked great questions.',
      },
    ],
  },
  {
    account: 'gmail',
    place: 'drafts',
    subject: 'Invitation copy for Dad',
    hoursAgo: 2,
    labels: ['Family'],
    messages: [
      {
        from: 'me',
        to: ['amara'],
        html: personal([
          'Draft text for the invitation:',
          '<i>Seventy years, one lake, all of us.</i>',
          'Too much?',
        ]),
      },
    ],
  },
  {
    account: 'gmail',
    place: 'drafts',
    subject: '',
    hoursAgo: 50,
    messages: [{ from: 'me', to: [], text: 'Note to self: ask Kenji about the' }],
  },
  {
    account: 'gmail',
    place: 'archive',
    labels: ['Travel'],
    subject: 'Hotel confirmation — Stockholm',
    hoursAgo: 700,
    messages: [
      {
        from: 'rail',
        html: receipt({
          vendor: 'Stays',
          orderId: 'ST-1182',
          lines: [{ item: '2 nights, Södermalm', amount: 'SEK 2,380' }],
          total: 'SEK 2,380',
        }),
      },
    ],
  },
  {
    account: 'gmail',
    place: 'archive',
    subject: 'Re: Old photos',
    hoursAgo: 800,
    messages: [
      { from: 'dad', text: 'I found the box with the Enugu photos. Will scan them.' },
      { from: 'me', to: ['dad'], gap: 2, text: 'Amazing, thank you Dad!' },
    ],
  },
  {
    account: 'gmail',
    place: 'archive',
    labels: ['Receipts'],
    subject: 'Your order LB-54001',
    hoursAgo: 1000,
    noBody: true,
    messages: [
      {
        from: 'bookshop',
        html: receipt({
          vendor: 'Lantern Books',
          orderId: 'LB-54001',
          lines: [{ item: 'The Dispossessed — Ursula K. Le Guin', amount: '€12.50' }],
          total: '€12.50',
        }),
      },
    ],
  },
  {
    account: 'gmail',
    place: 'archive',
    subject: 'Running club — spring schedule',
    hoursAgo: 500,
    messages: [
      {
        from: 'marcus',
        to: ['me', 'kenji', 'lena'],
        text: 'Tuesdays 7am at the lakes, Saturdays 9am long run. See you there.',
      },
    ],
  },
  {
    account: 'gmail',
    place: 'archive',
    subject: '[airpost/airpost] Release 0.3.0',
    hoursAgo: 340,
    messages: [
      {
        from: 'github',
        text: 'Release 0.3.0 published.\n\n- Rust engine is now the default\n- Faster search indexing\n\n—\nView it on GitHub.',
      },
    ],
  },
  {
    account: 'gmail',
    place: 'archive',
    labels: ['Family'],
    subject: 'Mum’s recipe book',
    hoursAgo: 1300,
    messages: [
      {
        from: 'amara',
        text: 'I scanned the recipe book! All 84 pages.',
        files: [pdf('recipes-scan.pdf', 18_400_000)],
      },
    ],
  },
  {
    account: 'gmail',
    place: 'spam',
    subject: 'Congratulations! You have been selected',
    hoursAgo: 15,
    unread: 1,
    messages: [{ from: 'spammer3', html: suspicious() }],
  },
  {
    account: 'gmail',
    place: 'spam',
    subject: 'Re: your invoice is overdue',
    hoursAgo: 100,
    unread: 1,
    messages: [
      {
        from: 'spammer1',
        text: 'Please open the attached invoice immediately.',
        files: [
          { filename: 'invoice.pdf.exe', contentType: 'application/octet-stream', size: 31_000 },
        ],
      },
    ],
  },
  {
    account: 'gmail',
    place: 'trash',
    subject: 'Flash sale ends tonight',
    hoursAgo: 55,
    messages: [
      {
        from: 'bookshop',
        html: newsletter({
          title: 'Lantern Books',
          issue: 'Flash sale',
          brandColor: '#3d405b',
          items: [{ heading: '30% off everything', text: 'Tonight only.' }],
        }),
      },
    ],
  },
  {
    account: 'gmail',
    place: 'trash',
    subject: 'Re: Saturday',
    hoursAgo: 260,
    messages: [{ from: 'lena', text: 'Never mind, it got cancelled.' }],
  },
  {
    account: 'gmail',
    place: 'trash',
    subject: 'Your weekly screen time report',
    hoursAgo: 170,
    messages: [
      { from: 'marcus', text: 'Just kidding, this is not a real report. Run with us on Saturday!' },
    ],
  },
];

/** Delivered one at a time by sync.wake or the mock's incoming timer. */
export const incoming: ThreadScript[] = [
  {
    account: 'gmail',
    place: 'inbox',
    subject: 'Quick question about June',
    hoursAgo: 0,
    unread: 1,
    messages: [
      {
        from: 'amara',
        text: 'Can you check if the lake house has room for 14? Uncle Emeka is bringing the cousins.',
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: 'Final package checklist',
    hoursAgo: 0,
    unread: 1,
    messages: [
      {
        from: 'priya',
        cc: ['jonas'],
        text: 'Here is what the board expects on the 21st: logo files, colour specs, type specimens, the icon set.',
      },
    ],
  },
  {
    account: 'imap',
    place: 'inbox',
    subject: '[Linear] AIRPOST-219 Search highlights are off by one',
    hoursAgo: 0,
    unread: 1,
    messages: [
      {
        from: 'linear',
        html: notification({
          service: 'Linear',
          headline: 'New issue assigned to you',
          body: 'Search result snippets highlight one character too many when the match is at the end of a line.',
          cta: 'Open issue',
          href: 'https://linear.example/airpost/issue/AIRPOST-219',
          accent: '#5e6ad2',
        }),
      },
    ],
  },
  {
    account: 'gmail',
    place: 'inbox',
    subject: 'Running this Saturday?',
    hoursAgo: 0,
    unread: 1,
    messages: [{ from: 'marcus', text: 'Long run Saturday, 15k. You in?' }],
  },
];
