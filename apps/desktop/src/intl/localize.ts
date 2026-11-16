import fs from 'fs';
import path from 'path';
import type { ReactNode } from 'react';
const React = process.type === 'renderer' ? require('react') : null;

const RTL_LANGS = [
  'ae' /* Avestan */,
  'ar' /* 'العربية', Arabic */,
  'arc' /* Aramaic */,
  'bcc' /* 'بلوچی مکرانی', Southern Balochi */,
  'bqi' /* 'بختياري', Bakthiari */,
  'ckb' /* 'Soranî / کوردی', Sorani */,
  'dv' /* Dhivehi */,
  'fa' /* 'فارسی', Persian */,
  'glk' /* 'گیلکی', Gilaki */,
  'he' /* 'עברית', Hebrew */,
  'ku' /* 'Kurdî / كوردی', Kurdish */,
  'mzn' /* 'مازِرونی', Mazanderani */,
  'nqo' /* N'Ko */,
  'pnb' /* 'پنجابی', Western Punjabi */,
  'ps' /* 'پښتو', Pashto, */,
  'sd' /* 'سنڌي', Sindhi */,
  'ug' /* 'Uyghurche / ئۇيغۇرچە', Uyghur */,
  'ur' /* 'اردو', Urdu */,
  'yi' /* 'ייִדיש', Yiddish */,
];

// For now, we only default to a localized version of Airpost if the translations
// have been manually reviewed by a contributor. We have Google translations in /tons/
// of languages but in many languages the translations are poor.
const VERIFIED_LANGS = [
  'en',
  'de',
  'es',
  'es_419',
  'fr',
  'ko',
  'pl',
  'ja',
  'zh-CN',
  'zh-TW',
  'pt',
  'pt-BR',
  'gl',
  'tr',
  'ru',
  'cs',
  'uk',
  'hu',
  'he',
];

const LANG_NAMES: { [code: string]: string } = {
  af: 'Afrikaans',
  am: 'Amharic',
  an: 'Aragonese',
  ar: 'Arabic',
  ast: 'Asturian',
  az: 'Azerbaijani',
  be: 'Belarusian',
  bg: 'Bulgarian',
  bn: 'Bengali',
  br: 'Breton',
  bs: 'Bosnian',
  ca: 'Catalan',
  cak: 'Kaqchikel',
  ceb: 'Cebuano',
  co: 'Corsican',
  cs: 'Czech',
  cy: 'Welsh',
  da: 'Danish',
  de: 'German',
  dsb: 'Lower Sorbian',
  el: 'Greek',
  en: 'English',
  eo: 'Esperanto',
  es: 'Spanish',
  es_419: 'Spanish - Latin America',
  et: 'Estonian',
  eu: 'Basque',
  fa: 'Persian',
  fi: 'Finnish',
  fr: 'French',
  fy: 'Western Frisian',
  ga: 'Irish',
  gd: 'Gaelic',
  gl: 'Galician',
  gu: 'Gujarati',
  ha: 'Hausa',
  haw: 'Hawaiian',
  he: 'Hebrew',
  hi: 'Hindi',
  hmn: 'Hmong',
  hr: 'Croatian',
  hsb: 'Upper Sorbian',
  ht: 'Haitian',
  hu: 'Hungarian',
  hy: 'Armenian',
  id: 'Indonesian',
  ig: 'Igbo',
  is: 'Icelandic',
  it: 'Italian',
  ja: 'Japanese',
  ka: 'Georgian',
  kab: 'Kabyle',
  kk: 'Kazakh',
  km: 'Central Khmer',
  kn: 'Kannada',
  ko: 'Korean',
  ku: 'Kurdish',
  ky: 'Kirghiz',
  lb: 'Luxembourgish',
  lo: 'Lao',
  lt: 'Lithuanian',
  lv: 'Latvian',
  mg: 'Malagasy',
  mi: 'Maori',
  mk: 'Macedonian',
  mn: 'Mongolian',
  ms: 'Malay',
  mt: 'Maltese',
  my: 'Burmese',
  nb: 'Norwegian Bokmål',
  ne: 'Nepali',
  nl: 'Dutch',
  nr: 'Ndebele',
  nso: 'Northern Sotho',
  oc: 'Occitan',
  pa: 'Punjabi',
  pl: 'Polish',
  pt: 'Portuguese',
  'pt-BR': 'Brazilian Portuguese',
  rm: 'Romansh',
  ro: 'Romanian',
  ru: 'Russian',
  si: 'Sinhala',
  sk: 'Slovak',
  sl: 'Slovenian',
  sm: 'Samoan',
  so: 'Somali',
  sq: 'Albanian',
  sr: 'Serbian',
  ss: 'Swati',
  st: 'Southern Sotho',
  sv: 'Swedish',
  ta: 'Tamil',
  th: 'Thai',
  tn: 'Tswana',
  tr: 'Turkish',
  ts: 'Tsonga',
  uk: 'Ukrainian',
  uz: 'Uzbek',
  ve: 'Venda',
  vi: 'Vietnamese',
  xh: 'Xhosa',
  'yue-CN': 'Cantonese',
  'zh-CN': 'Chinese - Simplified',
  'zh-HK': 'Chinese - Hong Kong',
  'zh-TW': 'Chinese - Taiwan',
  zh: 'Chinese',
  zu: 'Zulu',
};

// The locale string "en-US" or "fr-FR", etc. provided by the system
const systemLocale =
  process.type === 'renderer' ? window.navigator.language : require('electron').app.getLocale();

// The locale Airpost will default to. We do not default to unverified translations
const automaticLocale =
  VERIFIED_LANGS.includes(systemLocale) || VERIFIED_LANGS.includes(systemLocale.split('-').shift())
    ? systemLocale
    : 'en-US';

const langsDir = path.join(__dirname, '..', '..', 'lang');
const langsFiles = fs
  .readdirSync(langsDir)
  .filter((f) => f.endsWith('.json'))
  .map((f) => f.replace('.json', ''));

let localizations: { [en: string]: string } = {};
// Set by initializeLocalization, which every process runs first.
let locale: string | null = null;

export let isRTL = false;

export function initializeLocalization({ configDirPath }: { configDirPath: string }) {
  let chosen = automaticLocale;

  // Load the user's language choice if they've explicitly set one in settings
  // Note we can't use our nice Config class becuase the config schema itself
  // contains localized strings...
  try {
    const config = JSON.parse(fs.readFileSync(path.join(configDirPath, 'config.json')).toString());
    const choice = config['*'].core.intl.language;
    if (choice && choice !== '') {
      // '' represents "use automatic"
      chosen = choice;
    }
  } catch (err) {
    // may be first run
  }
  locale = chosen;

  const lang = chosen.split('-')[0] || 'en';

  localizations = {};
  isRTL = false;

  // Load localizations for the base language (eg: `zh`)
  if (langsFiles.includes(lang)) {
    localizations = require(path.join(langsDir, `${lang}.json`));
    isRTL = RTL_LANGS.includes(lang);
  }

  // Load localizations for the full locale if present (eg: `zh-CN`)
  // and override base localizations with the more specific regional ones.
  if (langsFiles.includes(chosen)) {
    localizations = Object.assign(localizations, require(path.join(langsDir, `${chosen}.json`)));
  }
}

// A lookup for the language `choice` (a config `core.intl.language` value, '' for automatic)
// that leaves this process's localization alone: for text another process sends on the user's
// behalf, such as the engine's resurfacing messages, after a language change the windows only
// pick up on relaunch.
export function localizerFor(choice: string | null | undefined): (en: string) => string {
  const want = choice || automaticLocale;
  const lang = want.split('-')[0] || 'en';
  let table: { [en: string]: string } = {};
  if (langsFiles.includes(lang)) table = { ...require(path.join(langsDir, `${lang}.json`)) };
  if (langsFiles.includes(want)) {
    table = Object.assign(table, require(path.join(langsDir, `${want}.json`)));
  }
  return (en: string) => table[en] || en;
}

export function localized(en: string, ...subs: unknown[]): string {
  let i = 0;
  let translated = localizations[en] || en;
  if (subs.length) {
    // Support "%@ and %@" OR "%1$@ and %2$@" which allows for param reordering.
    // The translation may contain this format even if the original string does not.
    if (translated.includes('%1$@')) {
      subs.forEach((sub, idx) => {
        translated = translated.replace(`%${idx + 1}$@`, String(sub));
      });
    } else {
      translated = translated.replace(/%@/g, () => String(subs[i++]));
    }
  }
  return translated;
}

export function localizedReactFragment(en: string, ...subs: ReactNode[]) {
  let translated = localizations[en] || en;
  if (!subs.length) {
    return translated;
  }

  // Support "%@ and %@" OR "%1$@ and %2$@" which allows for param reordering.
  // The translation may contain this format even if the original string does not.

  const parts: ReactNode[] = [];
  let match = null;
  let used = 0;
  while ((match = /%(?:(\d+)\$)?@/g.exec(translated))) {
    if (match.index > 0) {
      parts.push(translated.substr(0, match.index));
    }
    if (match[1]) {
      parts.push(subs[Number(match[1]) - 1]);
    } else {
      parts.push(subs[used++]);
    }
    translated = translated.substr(match.index + match[0].length);
  }
  if (translated.length > 0) {
    parts.push(translated);
  }

  return React.createElement(React.Fragment, [], ...parts);
}

// For Preferences UI:

export function getCurrentLocale() {
  return locale;
}

export function getAvailableLanguages() {
  const localeToItem = (f: string) => ({ key: f, name: LANG_NAMES[f] || f });
  const current = locale;
  if (current === null) throw new Error('initializeLocalization() has not run');

  // The list we expose is mostly just languages, but also a few lang-locale combos like zh-CN.
  // If our current/system locale in a known combo expose that, otherwis cut it so it's always
  // a member of the available list.
  const currentLang = LANG_NAMES[current] ? current : current.split('-')[0];
  const automaticLang = LANG_NAMES[automaticLocale]
    ? automaticLocale
    : automaticLocale.split('-')[0];

  return {
    current: localeToItem(currentLang),
    automatic: localeToItem(automaticLang),
    verified: VERIFIED_LANGS.map(localeToItem),
    experimental: langsFiles.filter((f) => !VERIFIED_LANGS.includes(f)).map(localeToItem),
  };
}
