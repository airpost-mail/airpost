// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

/*
The contact editor's shape, which `contacts.get` answers and `contacts.create` and
`contacts.update` take. The engine maps it onto a CardDAV
vCard or a Google person (crates/tern-pim/src/contact.rs).
*/
export interface ContactBase {
  name: {
    displayName: string;
    familyName: string;
    givenName: string;
    honorificPrefix: string;
    honorificSuffix: string;
  };
  photoURL?: string;
  nicknames?: { value: string }[];
  company: string;
  title: string;
  phoneNumbers?: { value: string; type?: string }[];
  emailAddresses?: { value: string; type?: string }[];
  urls?: { value: string; type?: string }[];
  relations?: { person: string; type?: string }[];
  addresses?: {
    formattedValue: string;
    type?: string;
    city: string;
    country: string;
    postalCode: string;
    region: string;
    streetAddress: string;
    extendedAddress: string;
  }[];
  birthdays?: { date: { year: number; month: number; day: number } }[];
  notes?: string;
}

export interface ContactInteractorMetadata {
  origin: string;
  readonly: boolean;
}

export interface ContactParseResult {
  metadata: ContactInteractorMetadata;
  data: ContactBase;
}
