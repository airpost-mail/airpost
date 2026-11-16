#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors
//
// Turns schema/airpost-ipc.json (written by `cargo run -p airpost-ipc --bin airpost-ipc-schema`) into
// src/protocol/*.ts: the types with json-schema-to-typescript, the constants from `x-airpost`.
// With --check it writes nothing and exits 1 when a file differs, as CI runs it.
//
//   npm run generate     # both steps
//   npm run check        # both, as checks

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { compile } from 'json-schema-to-typescript';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const SCHEMA = path.join(ROOT, 'schema', 'airpost-ipc.json');
const OUT = path.join(ROOT, 'src', 'protocol');
const CHECK = process.argv.includes('--check');

const BANNER = `// SPDX-License-Identifier: GPL-3.0-only
// Generated from crates/airpost-ipc by packages/client/scripts/generate.mjs. Do not edit: change
// the Rust types, then run \`npm run generate\` in packages/client.
`;

const doc = JSON.parse(fs.readFileSync(SCHEMA, 'utf8'));
const defs = doc.$defs;

// A `$ref` with siblings (a field's doc comment) makes json-schema-to-typescript declare the
// referenced type again under a new name (`AccountJson1`). Name the type instead, keeping the
// doc comment.
function nameRefs(node) {
  if (Array.isArray(node)) return node.map(nameRefs);
  if (!node || typeof node !== 'object') return node;
  const out = {};
  for (const [k, v] of Object.entries(node)) out[k] = nameRefs(v);
  if (typeof out.$ref === 'string' && Object.keys(out).length > 1) {
    const name = out.$ref.replace('#/$defs/', '');
    delete out.$ref;
    out.tsType = name;
  }
  // A `serde_json::Value` with a doc comment is annotations only; json-schema-to-typescript
  // would read it as an object.
  if (Object.keys(out).length && Object.keys(out).every((k) => ANNOTATIONS.has(k))) {
    out.tsType = 'unknown';
  }
  return out;
}

const ANNOTATIONS = new Set(['description', 'default', 'title', 'examples', 'deprecated']);

async function types() {
  const root = {
    title: 'AirpostIpcRoot',
    type: 'object',
    properties: {},
    additionalProperties: false,
    $defs: nameRefs(defs),
  };
  const ts = await compile(root, 'AirpostIpcRoot', {
    bannerComment: '',
    additionalProperties: false,
    unreachableDefinitions: true,
    format: true,
    style: { singleQuote: true, printWidth: 100, trailingComma: 'es5' },
  });
  const body = ts
    .replace(/(\/\*\*(?:(?!\*\/)[\s\S])*\*\/\n)?export interface AirpostIpcRoot \{\}\n\n?/, '')
    // json-schema-to-typescript's note on every $def.
    .replace(/\/\*\*\n \* This (?:interface|type) was referenced by `AirpostIpcRoot`'s JSON-Schema\n \* via the `definition` "[^"]+"\.\n \*\/\n/g, '')
    .replace(/\n \*\n \* This (?:interface|type) was referenced by `AirpostIpcRoot`'s JSON-Schema\n \* via the `definition` "[^"]+"\./g, '');
  const dup = body.match(/^export (?:interface|type) ([A-Za-z]+\d+)\b/m);
  if (dup) throw new Error(`json-schema-to-typescript declared ${dup[1]}: a $def is reached twice`);
  return `${BANNER}\n${body}`;
}

// `ACCOUNT_OAUTH_BEGIN` -> `AccountOAuthBegin`.
const WORDS = { OAUTH: 'OAuth' };
function pascal(screaming) {
  return screaming
    .split('_')
    .map((w) => WORDS[w] ?? w.charAt(0) + w.slice(1).toLowerCase())
    .join('');
}

function jsdoc(text, indent = '') {
  if (!text) return '';
  const lines = text.split('\n');
  if (lines.length === 1) return `${indent}/** ${lines[0]} */\n`;
  return `${indent}/**\n${lines.map((l) => `${indent} *${l ? ` ${l}` : ''}`).join('\n')}\n${indent} */\n`;
}

// airpost-ipc module -> TS name and what it is.
const GROUPS = {
  method: ['Method', 'Method and notification names.'],
  feature: [
    'Feature',
    'What `hello.features` may list. Clients gate on these, never on versions.',
  ],
  code: ['ErrorCode', 'Error codes: the standard JSON-RPC codes, then the engine\'s from -32000 down.'],
  account_state: [
    'AccountState',
    'An account\'s `state` in `hello`, `engine.status`, `account.list` and `account.state`.',
  ],
  capability_reason: ['CapabilityReason', 'Why a capability is unavailable (`Capability.reason`).'],
  integration_kind: ['IntegrationKind', 'The kinds in `Capabilities.integrations`.'],
  server_feature: ['ServerFeature', 'The features in `Capabilities.server`.'],
  ai_feature: ['AiFeature', 'The features in `Capabilities.ai`.'],
  unsubscribe_method: ['UnsubscribeMethod', '`messages.unsubscribe` methods.'],
  draft_kind: ['DraftKind', '`draft.create` kinds.'],
  task_status: ['TaskStatus', 'A task\'s `status`.'],
  placement_flag: ['PlacementFlag', 'The bits of `MessageJson.folders` values.'],
};

function literal(v) {
  return typeof v === 'string' ? `'${v.replace(/\\/g, '\\\\').replace(/'/g, "\\'")}'` : String(v);
}

function constants(typeNames) {
  const x = doc['x-airpost'];
  let out = `${BANNER}\n`;
  for (const [mod, [name, about]] of Object.entries(GROUPS)) {
    const items = x.groups[mod];
    if (!items) throw new Error(`no group ${mod} in the schema`);
    if (typeNames.has(name)) throw new Error(`${name} is both a constant group and a type`);
    out += jsdoc(`${about} (airpost-ipc \`${mod}\`)`);
    out += `export const ${name} = {\n`;
    for (const item of items) {
      out += jsdoc(item.description, '  ');
      out += `  ${pascal(item.name)}: ${literal(item.value)},\n`;
    }
    out += `} as const;\n`;
    out += `export type ${name} = (typeof ${name})[keyof typeof ${name}];\n\n`;
  }
  out += `/** The protocol version \`hello\` names. */\n`;
  for (const [k, v] of Object.entries(x.constants)) {
    out += `export const ${k} = ${literal(v)};\n`;
  }
  out += `\n/** Every key \`settings.get\` and \`settings.set\` take, with the JSON type of its value. */\n`;
  out += `export const SETTINGS_KEYS = {\n`;
  for (const [k, v] of Object.entries(x.settingsKeys)) out += `  ${literal(k)}: ${literal(v)},\n`;
  out += `} as const;\nexport type SettingsKey = keyof typeof SETTINGS_KEYS;\n\n`;
  out += `/**\n * Who may call each method: \`open\` (any connection), \`local-only\` (local clients and\n * relayed devices with the admin scope), \`parent-only\` (the engine's parent), \`debug\`.\n */\n`;
  out += `export const METHOD_CLASSES = {\n`;
  for (const [k, v] of Object.entries(x.methodClasses)) out += `  ${literal(k)}: ${literal(v)},\n`;
  out += `} as const;\n`;
  return out;
}

const INDEX = `${BANNER}
import type { RpcMethods, RpcNotifications } from './types';

export * from './types';
export * from './constants';

export type RpcMethodName = keyof RpcMethods;
export type RpcParams<M extends RpcMethodName> = RpcMethods[M]['params'];
export type RpcResult<M extends RpcMethodName> = RpcMethods[M]['result'];
/** Lets \`call('engine.status')\` leave out params while every other method takes them. */
export type RpcParamsArg<M extends RpcMethodName> =
  RpcParams<M> extends undefined ? [params?: undefined] : [params: RpcParams<M>];
export type RpcNotificationName = keyof RpcNotifications;
/** A notification as a tagged union, for a \`switch (n.method)\`. */
export type EngineNotification = {
  [K in RpcNotificationName]: { method: K; params: RpcNotifications[K] };
}[RpcNotificationName];
`;

const typesText = await types();
const typeNames = new Set(
  [...typesText.matchAll(/^export (?:interface|type) ([A-Za-z0-9_]+)/gm)].map((m) => m[1])
);
const files = {
  'types.ts': typesText,
  'constants.ts': constants(typeNames),
  'index.ts': INDEX,
};

let stale = false;
fs.mkdirSync(OUT, { recursive: true });
for (const [name, text] of Object.entries(files)) {
  const file = path.join(OUT, name);
  const current = fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : '';
  if (current === text) continue;
  const shown = path.relative(path.resolve(ROOT, '..', '..'), file);
  if (CHECK) {
    console.error(`${shown} is out of date: run \`npm run generate\` in packages/client`);
    stale = true;
  } else {
    fs.writeFileSync(file, text);
    console.error(`wrote ${shown}`);
  }
}
for (const name of fs.readdirSync(OUT)) {
  if (!(name in files)) {
    console.error(`${path.join('src', 'protocol', name)} isn't generated; remove it`);
    stale = true;
  }
}
process.exit(stale ? 1 : 0);
