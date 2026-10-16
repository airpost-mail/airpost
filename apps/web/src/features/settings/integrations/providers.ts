// Integration providers. The engine's integrations crate makes the provider calls; the web
// client only collects settings and never talks to a provider itself.

export type IntegrationKind = 'translation' | 'grammar' | 'enrichment';
export type ProviderId = 'libretranslate' | 'deepl' | 'languagetool' | 'pdl';
export type FieldKey = 'url' | 'apiKey' | 'username';

export interface ProviderField {
  key: FieldKey;
  label: string;
  /** Secrets go to the engine and are never written to browser storage. */
  secret: boolean;
  required: boolean;
  placeholder?: string;
  defaultValue?: string;
}

export interface ProviderDef {
  id: ProviderId;
  name: string;
  kind: IntegrationKind;
  blurb: string;
  privacy: string;
  fields: ProviderField[];
}

export const PROVIDERS: ProviderDef[] = [
  {
    id: 'libretranslate',
    name: 'LibreTranslate',
    kind: 'translation',
    blurb: 'Open-source translation you can host yourself.',
    privacy: 'Message text goes to the LibreTranslate server at this address.',
    fields: [
      {
        key: 'url',
        label: 'Server URL',
        secret: false,
        required: true,
        placeholder: 'https://translate.example.org',
      },
      { key: 'apiKey', label: 'API key', secret: true, required: false, placeholder: 'Optional' },
    ],
  },
  {
    id: 'deepl',
    name: 'DeepL',
    kind: 'translation',
    blurb: 'Translation through the DeepL API, free or Pro key.',
    privacy: 'Message text goes to DeepL under your key.',
    fields: [{ key: 'apiKey', label: 'API key', secret: true, required: true }],
  },
  {
    id: 'languagetool',
    name: 'LanguageTool',
    kind: 'grammar',
    blurb: 'Grammar and style checks while you write.',
    privacy: 'Draft text goes to the LanguageTool server at this address as you type.',
    fields: [
      {
        key: 'url',
        label: 'Server URL',
        secret: false,
        required: true,
        defaultValue: 'https://api.languagetool.org',
      },
      {
        key: 'username',
        label: 'Premium username',
        secret: false,
        required: false,
        placeholder: 'Optional',
      },
      {
        key: 'apiKey',
        label: 'Premium API key',
        secret: true,
        required: false,
        placeholder: 'Optional',
      },
    ],
  },
  {
    id: 'pdl',
    name: 'People Data Labs',
    kind: 'enrichment',
    blurb: 'Company and role details for the people who write to you.',
    privacy: 'Sender email addresses go to People Data Labs for a profile lookup.',
    fields: [{ key: 'apiKey', label: 'API key', secret: true, required: true }],
  },
];

export const KIND_LABEL: Record<IntegrationKind, string> = {
  translation: 'Translation',
  grammar: 'Grammar',
  enrichment: 'Contact enrichment',
};

export function providerDef(id: ProviderId): ProviderDef {
  return PROVIDERS.find((p) => p.id === id)!;
}

export function providersFor(kind: IntegrationKind): ProviderDef[] {
  return PROVIDERS.filter((p) => p.kind === kind);
}
