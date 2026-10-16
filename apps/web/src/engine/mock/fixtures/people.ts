import type { ContactRef } from '../../types';

export const ME_GMAIL: ContactRef = { name: 'Maya Okafor', email: 'maya.okafor@gmail.com' };
export const ME_IMAP: ContactRef = { name: 'Maya Okafor', email: 'maya@okafor.studio' };

export const people = {
  jonas: { name: 'Jonas Lindqvist', email: 'jonas@northwind.design' },
  priya: { name: 'Priya Raman', email: 'priya.raman@northwind.design' },
  theo: { name: 'Théo Marchand', email: 'theo@marchand-atelier.fr' },
  amara: { name: 'Amara Okafor', email: 'amara.okafor@gmail.com' },
  dad: { name: 'Chidi Okafor', email: 'chidi.okafor@outlook.com' },
  lena: { name: 'Lena Hoffmann', email: 'lena.hoffmann@posteo.de' },
  sam: { name: 'Sam Whitfield', email: 'sam@whitfield.io' },
  kenji: { name: 'Kenji Watanabe', email: 'kenji.w@fastmail.com' },
  rosa: { name: 'Rosa Delgado', email: 'rosa@tidepool.coop' },
  omar: { name: 'Omar Haddad', email: 'omar.haddad@harbourlabs.com' },
  ines: { name: 'Inês Carvalho', email: 'ines@harbourlabs.com' },
  felix: { name: 'Felix Brandt', email: 'felix@brandt.law' },
  noor: { name: 'Noor Siddiqui', email: 'noor.siddiqui@uni-leiden.nl' },
  marcus: { name: 'Marcus Bell', email: 'marcus.bell@icloud.com' },
  hana: { name: 'Hana Novak', email: 'hana@novak.cz' },
  github: { name: 'GitHub', email: 'notifications@github.com' },
  linear: { name: 'Linear', email: 'notifications@linear.app' },
  figma: { name: 'Figma', email: 'no-reply@figma.com' },
  stripe: { name: 'Stripe', email: 'receipts@stripe.com' },
  rail: { name: 'Nordic Rail', email: 'tickets@nordicrail.example' },
  airline: { name: 'Fjord Air', email: 'bookings@fjordair.example' },
  bank: { name: 'Harbour Bank', email: 'statements@harbourbank.example' },
  bookshop: { name: 'Lantern Books', email: 'orders@lanternbooks.example' },
  weekly: { name: 'The Interface Weekly', email: 'hello@interfaceweekly.example' },
  rustnews: { name: 'This Week in Rust', email: 'twir@rustnews.example' },
  landlord: { name: 'Keystone Property', email: 'tenants@keystoneproperty.example' },
  dentist: { name: 'Lindenhof Dental', email: 'appointments@lindenhof-dental.example' },
  spammer1: { name: 'Crypto Wins', email: 'win@cl4im-now.example' },
  spammer2: { name: 'Pharmacy Direct', email: 'deals@rx-cheap.example' },
  spammer3: { name: 'Prize Desk', email: 'prize@lucky-draw.example' },
} satisfies Record<string, ContactRef>;

export type PersonKey = keyof typeof people;
